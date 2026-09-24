#import <Foundation/Foundation.h>
#import <AppKit/AppKit.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>
#import <CoreGraphics/CoreGraphics.h>
#import <CoreVideo/CoreVideo.h>
#import <math.h>
#import <unistd.h>

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
    int _state; // 0 starting, 1 running, 2 stopping, 3 stopped, 4 error
    NSString *_error;
}
- (void)begin;
- (void)stop;
- (int)stateWithMessage:(char *)message capacity:(size_t)capacity;
- (CVPixelBufferRef)takeFrame;
@end

@implementation IOCapture
- (instancetype)init {
    if ((self = [super init])) {
        _lock = [NSLock new];
        _control = dispatch_queue_create("com.iotensity.capture.control", DISPATCH_QUEUE_SERIAL);
        _frames = dispatch_queue_create("com.iotensity.capture.frames", DISPATCH_QUEUE_SERIAL);
        _state = 0;
    }
    return self;
}
- (BOOL)cancelled {
    [_lock lock]; BOOL result = _cancelled; [_lock unlock]; return result;
}
- (void)setState:(int)state message:(NSString *)message {
    [_lock lock];
    _state = state;
    _error = message;
    if (state == 3 || state == 4) {
        if (_latest) CVPixelBufferRelease(_latest);
        _latest = NULL;
    }
    [_lock unlock];
}
- (void)fail:(NSString *)message {
    [self setState:4 message:message];
    // Called on the lifecycle queue. Release SCStream's output references.
    if (_stream) {
        SCStream *stream = _stream;
        _stream = nil;
        [stream stopCaptureWithCompletionHandler:^(NSError *error) { (void)error; }];
    }
}
- (void)begin {
    // ScreenCaptureKit owns the permission request. Legacy CoreGraphics preflight
    // can disagree with SCK grants after local rebuilds, so SCK's result is authoritative.
    dispatch_async(_control, ^{
        if ([self cancelled]) return;
            [SCShareableContent getShareableContentExcludingDesktopWindows:NO onScreenWindowsOnly:NO completionHandler:^(SCShareableContent *content, NSError *error) {
                dispatch_async(self->_control, ^{
                    if ([self cancelled]) return;
                    if (error || !content) {
                        NSString *message = error.code == SCStreamErrorUserDeclined
                            ? @"Screen Recording permission is required. Enable IOTensity in System Settings → Privacy & Security → Screen & System Audio Recording, then restart IOTensity and try again."
                            : error.localizedDescription ?: @"Could not enumerate the main display.";
                        [self fail:message]; return;
                    }
                    SCDisplay *display = nil;
                    for (SCDisplay *candidate in content.displays) {
                        if (candidate.displayID == CGMainDisplayID()) { display = candidate; break; }
                    }
                    if (!display) { [self fail:@"The main display is unavailable. Stop and restart capture after reconnecting it."]; return; }
                    NSMutableArray<SCRunningApplication *> *excluded = [NSMutableArray new];
                    for (SCRunningApplication *application in content.applications) {
                        if (application.processID == getpid()) [excluded addObject:application];
                    }
                    if (excluded.count == 0) { [self fail:@"Could not exclude IOTensity from capture. Restart the app and try again."]; return; }
                    // Application exclusion also covers an overlay created after capture starts.
                    SCContentFilter *filter = [[SCContentFilter alloc] initWithDisplay:display excludingApplications:excluded exceptingWindows:@[]];
                    SCStreamConfiguration *config = [SCStreamConfiguration new];
                    // Use the selected content's oriented size and backing scale,
                    // including portrait/Retina displays. Rust derives the small
                    // analysis image from the actual buffer after linear decoding.
                    if (@available(macOS 14.0, *)) {
                        config.width = MAX(1, llround(CGRectGetWidth(filter.contentRect) * filter.pointPixelScale));
                        config.height = MAX(1, llround(CGRectGetHeight(filter.contentRect) * filter.pointPixelScale));
                    } else {
                        CGDisplayModeRef mode = CGDisplayCopyDisplayMode(display.displayID);
                        size_t width = mode ? CGDisplayModeGetPixelWidth(mode) : display.width;
                        size_t height = mode ? CGDisplayModeGetPixelHeight(mode) : display.height;
                        if (mode) CGDisplayModeRelease(mode);
                        // A display mode can describe the unrotated panel. Match
                        // ScreenCaptureKit's display orientation before streaming.
                        if ((width > height) != (display.width > display.height)) {
                            size_t swap = width; width = height; height = swap;
                        }
                        config.width = width;
                        config.height = height;
                    }
                    config.pixelFormat = kCVPixelFormatType_32BGRA;
                    config.colorSpaceName = kCGColorSpaceSRGB;
                    config.minimumFrameInterval = CMTimeMake(1, 30);
                    config.queueDepth = 3;
                    config.showsCursor = NO;
                    if (@available(macOS 13.0, *)) config.capturesAudio = NO;
                    if (@available(macOS 14.0, *)) config.preservesAspectRatio = YES;
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
                            if (startError) [self fail:startError.localizedDescription];
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
    if (type != SCStreamOutputTypeScreen || !CMSampleBufferIsValid(sample)) return;
    CFArrayRef attachments = CMSampleBufferGetSampleAttachmentsArray(sample, false);
    if (!attachments || CFArrayGetCount(attachments) == 0) return;
    NSDictionary *metadata = (__bridge NSDictionary *)CFArrayGetValueAtIndex(attachments, 0);
    NSNumber *status = metadata[SCStreamFrameInfoStatus];
    if (!status || status.integerValue != SCFrameStatusComplete) return;
    CVPixelBufferRef buffer = CMSampleBufferGetImageBuffer(sample);
    if (!buffer || CVPixelBufferGetPixelFormatType(buffer) != kCVPixelFormatType_32BGRA) return;
    [_lock lock];
    if (!_cancelled && _state != 4) {
        CVPixelBufferRetain(buffer);
        if (_latest) CVPixelBufferRelease(_latest);
        _latest = buffer;
    }
    [_lock unlock];
}
- (void)stream:(SCStream *)stream didStopWithError:(NSError *)error {
    (void)stream;
    dispatch_async(_control, ^{
        if (![self cancelled]) [self fail:error.localizedDescription];
    });
}
- (int)stateWithMessage:(char *)message capacity:(size_t)capacity {
    [_lock lock];
    int state = _state;
    if (capacity) snprintf(message, capacity, "%s", _error.UTF8String ?: "");
    [_lock unlock];
    return state;
}
- (CVPixelBufferRef)takeFrame {
    [_lock lock];
    CVPixelBufferRef result = _latest;
    _latest = NULL;
    [_lock unlock];
    return result; // Ownership moves to Rust's synchronous callback wrapper.
}
- (void)dealloc { if (_latest) CVPixelBufferRelease(_latest); }
@end

void *io_capture_start(void) {
    @autoreleasepool {
        IOCapture *capture = [IOCapture new];
        [capture begin];
        return (__bridge_retained void *)capture;
    }
}
void io_capture_stop(void *handle) { [(__bridge IOCapture *)handle stop]; }
int io_capture_state(void *handle, char *message, size_t capacity) {
    @autoreleasepool { return [(__bridge IOCapture *)handle stateWithMessage:message capacity:capacity]; }
}
void io_capture_frame(void *handle, void *context, void (*receive)(void *, const unsigned char *, size_t, size_t, size_t)) {
    @autoreleasepool {
        CVPixelBufferRef frame = [(__bridge IOCapture *)handle takeFrame];
        if (!frame) return;
        if (CVPixelBufferLockBaseAddress(frame, kCVPixelBufferLock_ReadOnly) == kCVReturnSuccess) {
            receive(context, CVPixelBufferGetBaseAddress(frame), CVPixelBufferGetWidth(frame), CVPixelBufferGetHeight(frame), CVPixelBufferGetBytesPerRow(frame));
            CVPixelBufferUnlockBaseAddress(frame, kCVPixelBufferLock_ReadOnly);
        }
        CVPixelBufferRelease(frame);
    }
}
void io_capture_release(void *handle) {
    @autoreleasepool {
        IOCapture *capture = (__bridge_transfer IOCapture *)handle;
        [capture stop];
    }
}
