// Exercise the actual callback bridge without starting a capture or requesting
// permission. Rust covers elapsed-time policy; this covers native status/payload
// validation and lifecycle races using real CoreMedia samples.
#import "../src/sync/capture.m"
#import <assert.h>
#import <string.h>

static CMSampleBufferRef statusSample(NSNumber *status) {
    CMSampleBufferRef sample = NULL;
    assert(CMSampleBufferCreate(kCFAllocatorDefault, NULL, true, NULL, NULL, NULL,
                               1, 0, NULL, 0, NULL, &sample) == noErr);
    if (status) {
        CFArrayRef attachments = CMSampleBufferGetSampleAttachmentsArray(sample, true);
        NSMutableDictionary *metadata = (__bridge NSMutableDictionary *)CFArrayGetValueAtIndex(attachments, 0);
        metadata[SCStreamFrameInfoStatus] = status;
    }
    return sample;
}

static CMSampleBufferRef imageSample(OSType pixelFormat) {
    CVPixelBufferRef image = NULL;
    assert(CVPixelBufferCreate(kCFAllocatorDefault, 2, 2, pixelFormat, NULL, &image) == kCVReturnSuccess);
    CMVideoFormatDescriptionRef format = NULL;
    assert(CMVideoFormatDescriptionCreateForImageBuffer(kCFAllocatorDefault, image, &format) == noErr);
    CMSampleTimingInfo timing = { CMTimeMake(1, 30), kCMTimeZero, kCMTimeInvalid };
    CMSampleBufferRef sample = NULL;
    assert(CMSampleBufferCreateReadyWithImageBuffer(kCFAllocatorDefault, image, format, &timing, &sample) == noErr);
    CFArrayRef attachments = CMSampleBufferGetSampleAttachmentsArray(sample, true);
    NSMutableDictionary *metadata = (__bridge NSMutableDictionary *)CFArrayGetValueAtIndex(attachments, 0);
    metadata[SCStreamFrameInfoStatus] = @(SCFrameStatusComplete);
    CFRelease(format);
    CVPixelBufferRelease(image);
    return sample;
}

static void deliver(IOCapture *capture, CMSampleBufferRef sample) {
    // The callback deliberately never reads its stream parameter.
    [capture stream:(SCStream *)(id)capture didOutputSampleBuffer:sample ofType:SCStreamOutputTypeScreen];
}

static int state(IOCapture *capture) {
    char message[1024];
    return [capture stateWithMessage:message capacity:sizeof(message)];
}

int main(void) {
    @autoreleasepool {
        for (int value = SCFrameStatusIdle; value <= SCFrameStatusStopped; value++) {
            IOCapture *capture = [IOCapture new];
            CMSampleBufferRef sample = statusSample(@(value));
            assert(CMSampleBufferIsValid(sample));
            assert(!CMSampleBufferGetImageBuffer(sample));
            deliver(capture, sample);
            int lifecycle = -1, status = -1;
            double age = -1;
            char message[1024];
            CVPixelBufferRef frame = [capture takeFrameWithState:&lifecycle message:message capacity:sizeof(message) status:&status age:&age];
            assert(!frame && status == value && age >= 0);
            CFRelease(sample);
        }

        IOCapture *coalesced = [IOCapture new];
        CMSampleBufferRef complete = imageSample(kCVPixelFormatType_32BGRA);
        deliver(coalesced, complete);
        CFRelease(complete);
        CMSampleBufferRef idle = statusSample(@(SCFrameStatusIdle));
        deliver(coalesced, idle);
        CFRelease(idle);
        int lifecycle = -1, frameStatus = -1;
        double age = -1;
        char message[1024];
        CVPixelBufferRef pending = [coalesced takeFrameWithState:&lifecycle message:message capacity:sizeof(message) status:&frameStatus age:&age];
        assert(pending && frameStatus == SCFrameStatusIdle);
        CVPixelBufferRelease(pending);

        for (int malformed = 0; malformed < 6; malformed++) {
            IOCapture *capture = [IOCapture new];
            [capture setState:1 message:nil];
            CMSampleBufferRef idle = statusSample(@(SCFrameStatusIdle));
            deliver(capture, idle);
            CFRelease(idle);
            CMSampleBufferRef sample = malformed == 4 ? imageSample(kCVPixelFormatType_32ARGB) :
                statusSample(malformed == 1 ? @99 : malformed == 2 || malformed == 5 ? nil : @(SCFrameStatusComplete));
            if (malformed == 3) CMSampleBufferInvalidate(sample);
            if (malformed == 5) CMSampleBufferGetSampleAttachmentsArray(sample, true);
            deliver(capture, sample);
            assert(state(capture) == 4); // Idle cannot mask unusable new content.
            [capture setState:1 message:nil]; // Simulate a late start completion.
            assert(state(capture) == 4);
            CFRelease(sample);
        }

        IOCapture *cancelled = [IOCapture new];
        [cancelled stop];
        CMSampleBufferRef bad = statusSample(@(SCFrameStatusComplete));
        deliver(cancelled, bad);
        [cancelled setState:1 message:nil];
        int stoppedState = state(cancelled);
        assert(stoppedState == 2 || stoppedState == 3);
        CFRelease(bad);

        // Permission and a stop chosen in macOS are final; other errors retry.
        NSError *declined = [NSError errorWithDomain:SCStreamErrorDomain code:SCStreamErrorUserDeclined userInfo:nil];
        NSError *userStopped = [NSError errorWithDomain:SCStreamErrorDomain code:SCStreamErrorUserStopped userInfo:nil];
        NSError *internal = [NSError errorWithDomain:SCStreamErrorDomain code:SCStreamErrorInternalError userInfo:nil];
        for (NSError *error in @[declined, userStopped, internal]) {
            IOCapture *capture = [IOCapture new];
            [capture setState:1 message:nil];
            [capture failWithError:error fallback:@"Fallback."];
            int expected = error == internal ? 4 : 5;
            assert(state(capture) == expected);
            [capture setState:1 message:nil]; // A late start completion cannot revive it.
            [capture fail:@"Later transient error."];
            CMSampleBufferRef late = statusSample(@99);
            deliver(capture, late);
            CFRelease(late);
            assert(state(capture) == expected);
        }
        IOCapture *missing = [IOCapture new];
        [missing failWithError:nil fallback:@"Fallback."];
        char missingMessage[1024];
        assert([missing stateWithMessage:missingMessage capacity:sizeof(missingMessage)] == 4);
        assert(strcmp(missingMessage, "Fallback.") == 0);

        // The GPU-scaled capture keeps the display's aspect and never upscales.
        struct { double width, height; size_t expectedWidth, expectedHeight; } sizes[] = {
            {5120, 2880, 1024, 576}, {6016, 3384, 1024, 576}, {3024, 1964, 1024, 665},
            {1080, 1920, 576, 1024}, {3440, 1440, 1024, 429}, {800, 600, 800, 600}, {0, 0, 1, 1},
        };
        for (size_t i = 0; i < sizeof(sizes) / sizeof(sizes[0]); i++) {
            size_t width = 0, height = 0;
            captureSize(sizes[i].width, sizes[i].height, 1024, &width, &height);
            assert(width == sizes[i].expectedWidth && height == sizes[i].expectedHeight);
        }

        IOCapture *audio = [IOCapture new];
        [audio setState:1 message:nil];
        CMSampleBufferRef nonScreen = statusSample(nil);
        if (@available(macOS 13.0, *)) {
            [audio stream:(SCStream *)(id)audio didOutputSampleBuffer:nonScreen ofType:SCStreamOutputTypeAudio];
        }
        assert(state(audio) == 1); // Non-screen callbacks remain ignored.
        CFRelease(nonScreen);
    }
    return 0;
}
