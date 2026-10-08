#import <Foundation/Foundation.h>

// Keeps App Nap and timer coalescing from stretching the 30 FPS output sleeps
// past the 250 ms publisher and 1,000 ms firmware watchdogs while the app is
// hidden or occluded. UserInitiatedAllowingIdleSystemSleep is UserInitiated
// without IdleSystemSleepDisabled: it opts out of App Nap but still lets the
// Mac idle-sleep, which turns the lights off through the firmware watchdog.
// LatencyCritical requests precise timers for the short UDP frame interval.
void *io_activity_begin(const char *reason) {
    @autoreleasepool {
        NSString *text = (reason ? [NSString stringWithUTF8String:reason] : nil) ?: @"IOTensity light output";
        id<NSObject> activity = [[NSProcessInfo processInfo]
            beginActivityWithOptions:NSActivityUserInitiatedAllowingIdleSystemSleep | NSActivityLatencyCritical
            reason:text];
        return (__bridge_retained void *)activity;
    }
}

// Balances the retained token from io_activity_begin exactly once.
void io_activity_end(void *handle) {
    if (!handle) return;
    @autoreleasepool {
        id<NSObject> activity = (__bridge_transfer id<NSObject>)handle;
        [[NSProcessInfo processInfo] endActivity:activity];
    }
}
