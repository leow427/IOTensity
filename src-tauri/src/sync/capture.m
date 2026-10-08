#import <Foundation/Foundation.h>
#import <AppKit/AppKit.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>
#import <CoreGraphics/CoreGraphics.h>
#import <CoreVideo/CoreVideo.h>
#import <math.h>
#import <unistd.h>
#import <time.h>

static double monotonicSeconds(void) {
    struct timespec now;
    clock_gettime(CLOCK_MONOTONIC, &now);
    return now.tv_sec + now.tv_nsec / 1e9;
}

// Failures only the user can resolve. Restarting capture after them would
// prompt for permission again or override a stop chosen in macOS, so they end
// in state 5 instead of the retryable state 4. Codes are compared without the
// domain, as before: a misread foreign error only costs a manual restart.
static BOOL isPermanentCaptureError(NSError *error) {
    return error.code == SCStreamErrorUserDeclined || error.code == SCStreamErrorMissingEntitlements ||
           error.code == SCStreamErrorUserStopped;
}
static NSString *captureErrorMessage(NSError *error, NSString *fallback) {
    if (error.code == SCStreamErrorUserDeclined)
        return @"Screen Recording permission is required. Enable IOTensity in System Settings → Privacy & Security → Screen & System Audio Recording, then restart IOTensity and try again.";
    if (error.code == SCStreamErrorUserStopped)
        return @"Screen recording was stopped in macOS. Start Sync again to resume.";
    return error.localizedDescription ?: fallback;
}

// ScreenCaptureKit scales the display on the GPU; decoding every Retina pixel
// on the CPU costs tens of milliseconds per frame. One uniform scale, never
// upscaled, keeps the display's aspect to within rounding. Rust decodes this
// small frame to linear light and reduces it once more to the analysis image.
static void captureSize(double width, double height, double maxSide, size_t *outWidth, size_t *outHeight) {
    double scale = MIN(1.0, MAX(1.0, maxSide) / MAX(1.0, MAX(width, height)));
    *outWidth = (size_t)MAX(1, llround(width * scale));
    *outHeight = (size_t)MAX(1, llround(height * scale));
}

// The stream callback only retains the newest IOSurface-backed pixel buffer.
// All SCStream lifecycle operations are serialized; Rust owns the output clock.
@interface IOCapture : NSObject <SCStreamOutput, SCStreamDelegate> {
    NSLock *_lock;
    dispatch_queue_t _control;
    dispatch_queue_t _frames;
    SCStream *_stream;
    CVPixelBufferRef _latest;
    BOOL _cancelled;
    BOOL _startingStream;
    int _state; // 0 starting, 1 running, 2 stopping, 3 stopped, 4 error, 5 permanent error
    NSString *_error;
    NSInteger _frameStatus;
    double _lastActivity;
}
- (void)beginWithMaxSide:(double)maxSide;
- (void)stop;
- (int)stateWithMessage:(char *)message capacity:(size_t)capacity;
- (CVPixelBufferRef)takeFrameWithState:(int *)state message:(char *)message capacity:(size_t)capacity status:(int *)status age:(double *)age;
@end

@implementation IOCapture
- (instancetype)init {
    if ((self = [super init])) {
        _lock = [NSLock new];
        _control = dispatch_queue_create("com.iotensity.capture.control", DISPATCH_QUEUE_SERIAL);
        _frames = dispatch_queue_create("com.iotensity.capture.frames", DISPATCH_QUEUE_SERIAL);
        _state = 0;
        _frameStatus = -1;
    }
    return self;
}
- (BOOL)cancelled {
    [_lock lock]; BOOL result = _cancelled; [_lock unlock]; return result;
}
- (void)setState:(int)state message:(NSString *)message {
    [_lock lock];
    // A late start acknowledgement cannot revive a cancelled or failed source,
    // and a later transient error cannot hide a failure only the user can fix.
    if ((state == 1 && (_cancelled || _state >= 4)) || (state == 4 && _state == 5)) { [_lock unlock]; return; }
    _state = state;
    _error = message;
    if (state >= 3) {
        if (_latest) CVPixelBufferRelease(_latest);
        _latest = NULL;
    }
    [_lock unlock];
}
- (void)fail:(NSString *)message {
    [self fail:message permanent:NO];
}
- (void)failWithError:(NSError *)error fallback:(NSString *)fallback {
    [self fail:captureErrorMessage(error, fallback) permanent:isPermanentCaptureError(error)];
}
- (void)fail:(NSString *)message permanent:(BOOL)permanent {
    [self setState:(permanent ? 5 : 4) message:message];
    // Called on the lifecycle queue. Release SCStream's output references.
    if (_stream) {
        SCStream *stream = _stream;
        _stream = nil;
        [stream stopCaptureWithCompletionHandler:^(NSError *error) { (void)error; }];
    }
}
- (void)rejectFrame {
    NSString *message = @"Could not read the captured display frame.";
    [_lock lock];
    if (_cancelled || _state >= 4) { [_lock unlock]; return; }
    _state = 4;
    _error = message;
    if (_latest) CVPixelBufferRelease(_latest);
    _latest = NULL;
    [_lock unlock];
    // Publish Error immediately, but release SCStream on its lifecycle queue.
    dispatch_async(_control, ^{
        if (![self cancelled]) [self fail:message];
    });
}
- (void)beginWithMaxSide:(double)maxSide {
    // ScreenCaptureKit owns the permission request. Legacy CoreGraphics preflight
    // can disagree with SCK grants after local rebuilds, so SCK's result is authoritative.
    dispatch_async(_control, ^{
        if ([self cancelled]) return;
            [SCShareableContent getShareableContentExcludingDesktopWindows:NO onScreenWindowsOnly:NO completionHandler:^(SCShareableContent *content, NSError *error) {
                dispatch_async(self->_control, ^{
                    if ([self cancelled]) return;
                    if (error || !content) {
                        [self failWithError:error fallback:@"Could not enumerate the main display."]; return;
                    }
                    SCDisplay *display = nil;
                    for (SCDisplay *candidate in content.displays) {
                        if (candidate.displayID == CGMainDisplayID()) { display = candidate; break; }
                    }
                    if (!display) { [self fail:@"The main display is unavailable."]; return; }
                    NSMutableArray<SCRunningApplication *> *excluded = [NSMutableArray new];
                    for (SCRunningApplication *application in content.applications) {
                        if (application.processID == getpid()) [excluded addObject:application];
                    }
                    if (excluded.count == 0) { [self fail:@"Could not exclude IOTensity from capture. Restart the app and try again." permanent:YES]; return; }
                    // Application exclusion also covers an overlay created after capture starts.
                    SCContentFilter *filter = [[SCContentFilter alloc] initWithDisplay:display excludingApplications:excluded exceptingWindows:@[]];
                    SCStreamConfiguration *config = [SCStreamConfiguration new];
                    // Start from the selected content's oriented size and backing
                    // scale, including portrait/Retina displays.
                    double width, height;
                    if (@available(macOS 14.0, *)) {
                        width = CGRectGetWidth(filter.contentRect) * filter.pointPixelScale;
                        height = CGRectGetHeight(filter.contentRect) * filter.pointPixelScale;
                    } else {
                        CGDisplayModeRef mode = CGDisplayCopyDisplayMode(display.displayID);
                        size_t modeWidth = mode ? CGDisplayModeGetPixelWidth(mode) : display.width;
                        size_t modeHeight = mode ? CGDisplayModeGetPixelHeight(mode) : display.height;
                        if (mode) CGDisplayModeRelease(mode);
                        // A display mode can describe the unrotated panel. Match
                        // ScreenCaptureKit's display orientation before streaming.
                        if ((modeWidth > modeHeight) != (display.width > display.height)) {
                            size_t swap = modeWidth; modeWidth = modeHeight; modeHeight = swap;
                        }
                        width = (double)modeWidth;
                        height = (double)modeHeight;
                    }
                    size_t captureWidth, captureHeight;
                    captureSize(width, height, maxSide, &captureWidth, &captureHeight);
                    config.width = captureWidth;
                    config.height = captureHeight;
                    config.pixelFormat = kCVPixelFormatType_32BGRA;
                    config.colorSpaceName = kCGColorSpaceSRGB;
                    config.minimumFrameInterval = CMTimeMake(1, 30);
                    config.queueDepth = 3;
                    config.showsCursor = NO;
                    if (@available(macOS 13.0, *)) config.capturesAudio = NO;
                    // The requested size already has the display's aspect, so filling
                    // it stretches by under a pixel; letterboxing could add a dark
                    // edge and shift the normalized light mapping.
                    if (@available(macOS 14.0, *)) config.preservesAspectRatio = NO;
                    if (@available(macOS 15.0, *)) config.captureDynamicRange = SCCaptureDynamicRangeSDR;
                    self->_stream = [[SCStream alloc] initWithFilter:filter configuration:config delegate:self];
                    NSError *outputError = nil;
                    if (![self->_stream addStreamOutput:self type:SCStreamOutputTypeScreen sampleHandlerQueue:self->_frames error:&outputError]) {
                        [self fail:outputError.localizedDescription ?: @"Could not attach screen output."]; return;
                    }
                    self->_startingStream = YES;
                    [self->_stream startCaptureWithCompletionHandler:^(NSError *startError) {
                        dispatch_async(self->_control, ^{
                            self->_startingStream = NO;
                            if ([self cancelled]) { [self stopStream]; return; }
                            if (startError) [self failWithError:startError fallback:@"Could not start display capture."];
                            else [self setState:1 message:nil];
                        });
                    }];
                });
            }];
    });
}
- (void)stopStream {
    if (_startingStream) return; // Completion immediately stops a cancelled start.
    if (!_stream) { [self setState:3 message:nil]; return; }
    SCStream *stream = _stream;
    _stream = nil;
    [stream stopCaptureWithCompletionHandler:^(NSError *error) {
        dispatch_async(self->_control, ^{
            if (error) [self setState:4 message:[@"Could not stop capture: " stringByAppendingString:error.localizedDescription]];
            else [self setState:3 message:nil];
        });
    }];
}
- (void)stop {
    [_lock lock];
    if (_cancelled) { [_lock unlock]; return; }
    _cancelled = YES;
    _state = 2;
    if (_latest) CVPixelBufferRelease(_latest);
    _latest = NULL;
    [_lock unlock];
    dispatch_async(_control, ^{ [self stopStream]; });
}
- (void)stream:(SCStream *)stream didOutputSampleBuffer:(CMSampleBufferRef)sample ofType:(SCStreamOutputType)type {
    (void)stream;
    if (type != SCStreamOutputTypeScreen) return;
    if (!sample) { [self rejectFrame]; return; }
    CFArrayRef attachments = CMSampleBufferGetSampleAttachmentsArray(sample, false);
    if (!attachments || CFArrayGetCount(attachments) == 0) { [self rejectFrame]; return; }
    NSDictionary *metadata = (__bridge NSDictionary *)CFArrayGetValueAtIndex(attachments, 0);
    NSNumber *status = metadata[SCStreamFrameInfoStatus];
    if (![status isKindOfClass:[NSNumber class]]) { [self rejectFrame]; return; }
    NSInteger frameStatus = status.integerValue;
    CVPixelBufferRef buffer = NULL;
    if (frameStatus == SCFrameStatusComplete) {
        // Status-only samples carry no image; validate only the Complete payload.
        buffer = CMSampleBufferIsValid(sample) ? CMSampleBufferGetImageBuffer(sample) : NULL;
        if (!buffer || CVPixelBufferGetPixelFormatType(buffer) != kCVPixelFormatType_32BGRA) {
            // A malformed new frame must invalidate a preceding Idle state.
            [self rejectFrame];
            return;
        }
    } else if (frameStatus != SCFrameStatusIdle && frameStatus != SCFrameStatusBlank &&
               frameStatus != SCFrameStatusSuspended && frameStatus != SCFrameStatusStarted &&
               frameStatus != SCFrameStatusStopped) {
        [self rejectFrame]; return;
    }
    [_lock lock];
    if (!_cancelled && _state < 4) {
        _frameStatus = frameStatus;
        _lastActivity = monotonicSeconds();
        // Idle is positive evidence that the display is unchanged, even though
        // ScreenCaptureKit supplies no IOSurface. Retain a pending Complete frame.
        if (buffer) {
            CVPixelBufferRetain(buffer);
            if (_latest) CVPixelBufferRelease(_latest);
            _latest = buffer;
        } else if (frameStatus != SCFrameStatusIdle) {
            if (_latest) CVPixelBufferRelease(_latest);
            _latest = NULL;
        }
    }
    [_lock unlock];
}
- (void)stream:(SCStream *)stream didStopWithError:(NSError *)error {
    (void)stream;
    dispatch_async(_control, ^{
        if (![self cancelled]) [self failWithError:error fallback:@"Display capture stopped."];
    });
}
- (int)stateWithMessage:(char *)message capacity:(size_t)capacity {
    [_lock lock];
    int state = _state;
    if (capacity) snprintf(message, capacity, "%s", _error.UTF8String ?: "");
    [_lock unlock];
    return state;
}
- (CVPixelBufferRef)takeFrameWithState:(int *)state message:(char *)message capacity:(size_t)capacity status:(int *)status age:(double *)age {
    [_lock lock];
    *state = _state;
    *status = (int)_frameStatus;
    *age = _lastActivity > 0 ? MAX(0, monotonicSeconds() - _lastActivity) : -1;
    if (capacity) snprintf(message, capacity, "%s", _error.UTF8String ?: "");
    CVPixelBufferRef result = _latest;
    _latest = NULL;
    [_lock unlock];
    return result; // Ownership moves to Rust's synchronous callback wrapper.
}
- (void)dealloc { if (_latest) CVPixelBufferRelease(_latest); }
@end

void *io_capture_start(size_t max_side) {
    @autoreleasepool {
        IOCapture *capture = [IOCapture new];
        [capture beginWithMaxSide:(double)max_side];
        return (__bridge_retained void *)capture;
    }
}
void io_capture_stop(void *handle) { [(__bridge IOCapture *)handle stop]; }
int io_capture_state(void *handle, char *message, size_t capacity) {
    @autoreleasepool { return [(__bridge IOCapture *)handle stateWithMessage:message capacity:capacity]; }
}
int io_capture_poll(void *handle, char *message, size_t capacity, int *status, double *age, void *context, void (*receive)(void *, const unsigned char *, size_t, size_t, size_t)) {
    @autoreleasepool {
        int state = 0;
        CVPixelBufferRef frame = [(__bridge IOCapture *)handle takeFrameWithState:&state message:message capacity:capacity status:status age:age];
        if (!frame) return state;
        if (CVPixelBufferLockBaseAddress(frame, kCVPixelBufferLock_ReadOnly) == kCVReturnSuccess) {
            receive(context, CVPixelBufferGetBaseAddress(frame), CVPixelBufferGetWidth(frame), CVPixelBufferGetHeight(frame), CVPixelBufferGetBytesPerRow(frame));
            CVPixelBufferUnlockBaseAddress(frame, kCVPixelBufferLock_ReadOnly);
        } else {
            receive(context, NULL, 0, 0, 0);
        }
        CVPixelBufferRelease(frame);
        return state;
    }
}
void io_capture_release(void *handle) {
    @autoreleasepool {
        IOCapture *capture = (__bridge_transfer IOCapture *)handle;
        [capture stop];
    }
}
