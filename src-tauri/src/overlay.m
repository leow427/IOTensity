#import <AppKit/AppKit.h>
#import <objc/runtime.h>

// Keep Tauri's window/webview ownership intact. Host its content in a real
// nonactivating NSPanel rather than changing the class of Tauri's NSWindow.
// Every entry point and notification below runs on the AppKit main thread.
@interface IOOverlayPanel : NSPanel
@property(nonatomic, weak) NSWindow *owner;
@property(nonatomic, strong) id closeObserver;
@end

@implementation IOOverlayPanel
- (BOOL)canBecomeKeyWindow { return YES; }
- (BOOL)canBecomeMainWindow { return NO; }
- (void)sendEvent:(NSEvent *)event {
    // The borderless HTML titlebar is 40 points high. Handle its drag here,
    // since Tauri's drag command would move the hidden owning window instead.
    NSPoint point = event.locationInWindow;
    NSSize size = self.contentView.bounds.size;
    if (event.type == NSEventTypeLeftMouseDown && point.y >= size.height - 40
        && point.x < size.width - 44) {
        [self performWindowDragWithEvent:event];
        return;
    }
    [super sendEvent:event];
}
- (void)dealloc {
    if (_closeObserver) [[NSNotificationCenter defaultCenter] removeObserver:_closeObserver];
}
@end

static char IOOverlayPanelKey;

void io_overlay_show(void *pointer) {
    NSCAssert(NSThread.isMainThread, @"Overlay must be configured on the main thread");
    NSWindow *owner = (__bridge NSWindow *)pointer;
    IOOverlayPanel *panel = objc_getAssociatedObject(owner, &IOOverlayPanelKey);
    if (!panel) {
        panel = [[IOOverlayPanel alloc] initWithContentRect:owner.frame
            styleMask:NSWindowStyleMaskBorderless | NSWindowStyleMaskNonactivatingPanel
            backing:NSBackingStoreBuffered defer:NO];
        panel.owner = owner;
        panel.title = owner.title;
        panel.releasedWhenClosed = NO;
        panel.floatingPanel = YES;
        panel.becomesKeyOnlyIfNeeded = YES;
        panel.hidesOnDeactivate = NO;
        panel.level = NSScreenSaverWindowLevel;
        NSWindowCollectionBehavior behavior = NSWindowCollectionBehaviorCanJoinAllSpaces
            | NSWindowCollectionBehaviorFullScreenAuxiliary | NSWindowCollectionBehaviorStationary;
        if (@available(macOS 13.0, *)) behavior |= NSWindowCollectionBehaviorCanJoinAllApplications;
        panel.collectionBehavior = behavior;

        NSView *content = owner.contentView;
        owner.contentView = nil;
        panel.contentView = content;
        [owner orderOut:nil];
        __weak IOOverlayPanel *weakPanel = panel;
        panel.closeObserver = [[NSNotificationCenter defaultCenter]
            addObserverForName:NSWindowWillCloseNotification object:owner queue:nil
            usingBlock:^(NSNotification *notification) {
                IOOverlayPanel *closing = weakPanel;
                if (!closing) return;
                // Restore the webview before Tauri tears down its owning window.
                NSView *view = closing.contentView;
                closing.contentView = nil;
                closing.owner.contentView = view;
                [closing orderOut:nil];
                [closing close];
                objc_setAssociatedObject(notification.object, &IOOverlayPanelKey, nil, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
            }];
        objc_setAssociatedObject(owner, &IOOverlayPanelKey, panel, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    }
    [panel orderFrontRegardless];
}
