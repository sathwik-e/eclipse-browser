#import <Cocoa/Cocoa.h>
#import <QuartzCore/QuartzCore.h>
#import <WebKit/WebKit.h>
#include <stdlib.h>
#include <stdio.h>

typedef void (*EclipseMenuCallback)(const char *action);

@class EclipseMenuTarget;

static EclipseMenuCallback gCallback = NULL;
static EclipseMenuTarget *gMenuTarget = nil;

@interface EclipseMenuTarget : NSObject
- (void)performEclipseAction:(id)sender;
@end

@implementation EclipseMenuTarget
- (void)performEclipseAction:(id)sender {
    if (gCallback == NULL) return;
    NSString *action = nil;
    if ([sender isKindOfClass:[NSMenuItem class]]) {
        action = [(NSMenuItem *)sender representedObject];
    }
    if (![action isKindOfClass:[NSString class]] || action.length == 0) return;
    gCallback(action.UTF8String);
}
@end

static NSMenuItem *EclipseAddItem(NSMenu *menu,
                                  NSString *title,
                                  SEL action,
                                  NSString *key,
                                  NSEventModifierFlags modifiers,
                                  id target,
                                  NSString *identifier) {
    NSMenuItem *item = [[NSMenuItem alloc] initWithTitle:title
                                                  action:action
                                           keyEquivalent:(key != nil ? key : @"")];
    item.keyEquivalentModifierMask = modifiers;
    item.target = target;
    if (identifier != nil) {
        item.representedObject = identifier;
    }
    [menu addItem:item];
    return item;
}

static NSMenu *EclipseMenu(NSString *title) {
    return [[NSMenu alloc] initWithTitle:title];
}

void eclipse_install_menu(EclipseMenuCallback callback) {
    @autoreleasepool {
        [NSApplication sharedApplication];
        gCallback = callback;
        gMenuTarget = [[EclipseMenuTarget alloc] init];

        NSMenu *mainMenu = EclipseMenu(@"Main Menu");

        /* ------------------------------------------------------------- App */
        NSMenuItem *appMenuItem = [[NSMenuItem alloc] initWithTitle:@"Eclipse"
                                                             action:nil
                                                      keyEquivalent:@""];
        [mainMenu addItem:appMenuItem];
        NSMenu *appMenu = EclipseMenu(@"Eclipse");
        appMenuItem.submenu = appMenu;

        EclipseAddItem(appMenu, @"About Eclipse", @selector(orderFrontStandardAboutPanel:),
                       @"", 0, NSApp, nil);
        [appMenu addItem:[NSMenuItem separatorItem]];
        EclipseAddItem(appMenu, @"Settings…", @selector(performEclipseAction:), @",",
                       NSEventModifierFlagCommand, gMenuTarget, @"open_settings");
        [appMenu addItem:[NSMenuItem separatorItem]];

        NSMenuItem *servicesItem = [[NSMenuItem alloc] initWithTitle:@"Services"
                                                              action:nil
                                                       keyEquivalent:@""];
        NSMenu *servicesMenu = EclipseMenu(@"Services");
        servicesItem.submenu = servicesMenu;
        [appMenu addItem:servicesItem];
        [NSApp setServicesMenu:servicesMenu];

        [appMenu addItem:[NSMenuItem separatorItem]];
        EclipseAddItem(appMenu, @"Hide Eclipse", @selector(hide:), @"h",
                       NSEventModifierFlagCommand, NSApp, nil);
        EclipseAddItem(appMenu, @"Hide Others", @selector(hideOtherApplications:), @"h",
                       NSEventModifierFlagCommand | NSEventModifierFlagOption, NSApp, nil);
        EclipseAddItem(appMenu, @"Show All", @selector(unhideAllApplications:), @"",
                       0, NSApp, nil);
        [appMenu addItem:[NSMenuItem separatorItem]];
        EclipseAddItem(appMenu, @"Quit Eclipse", @selector(terminate:), @"q",
                       NSEventModifierFlagCommand, NSApp, nil);

        /* ------------------------------------------------------------ File */
        NSMenuItem *fileMenuItem = [[NSMenuItem alloc] initWithTitle:@"File"
                                                              action:nil
                                                       keyEquivalent:@""];
        [mainMenu addItem:fileMenuItem];
        NSMenu *fileMenu = EclipseMenu(@"File");
        fileMenuItem.submenu = fileMenu;

        EclipseAddItem(fileMenu, @"New Tab", @selector(performEclipseAction:), @"t",
                       NSEventModifierFlagCommand, gMenuTarget, @"new_tab");
        EclipseAddItem(fileMenu, @"Close Tab", @selector(performEclipseAction:), @"w",
                       NSEventModifierFlagCommand, gMenuTarget, @"close_tab");
        EclipseAddItem(fileMenu, @"Reopen Closed Tab", @selector(performEclipseAction:), @"t",
                       NSEventModifierFlagCommand | NSEventModifierFlagShift, gMenuTarget,
                       @"reopen_closed_tab");
        [fileMenu addItem:[NSMenuItem separatorItem]];
        EclipseAddItem(fileMenu, @"Open Location…", @selector(performEclipseAction:), @"l",
                       NSEventModifierFlagCommand, gMenuTarget, @"focus_omnibar");
        EclipseAddItem(fileMenu, @"Bookmark Tab", @selector(performEclipseAction:), @"d",
                       NSEventModifierFlagCommand, gMenuTarget, @"favorite_tab");
        [fileMenu addItem:[NSMenuItem separatorItem]];
        EclipseAddItem(fileMenu, @"Print…", @selector(performEclipseAction:), @"p",
                       NSEventModifierFlagCommand, gMenuTarget, @"print");
        [fileMenu addItem:[NSMenuItem separatorItem]];
        EclipseAddItem(fileMenu, @"Close Window", @selector(performEclipseAction:), @"w",
                       NSEventModifierFlagCommand | NSEventModifierFlagShift, gMenuTarget,
                       @"close_window");

        /* ------------------------------------------------------------ Edit */
        NSMenuItem *editMenuItem = [[NSMenuItem alloc] initWithTitle:@"Edit"
                                                              action:nil
                                                       keyEquivalent:@""];
        [mainMenu addItem:editMenuItem];
        NSMenu *editMenu = EclipseMenu(@"Edit");
        editMenuItem.submenu = editMenu;

        EclipseAddItem(editMenu, @"Undo", @selector(undo:), @"z",
                       NSEventModifierFlagCommand, nil, nil);
        EclipseAddItem(editMenu, @"Redo", @selector(redo:), @"z",
                       NSEventModifierFlagCommand | NSEventModifierFlagShift, nil, nil);
        [editMenu addItem:[NSMenuItem separatorItem]];
        EclipseAddItem(editMenu, @"Cut", @selector(cut:), @"x",
                       NSEventModifierFlagCommand, nil, nil);
        EclipseAddItem(editMenu, @"Copy", @selector(copy:), @"c",
                       NSEventModifierFlagCommand, nil, nil);
        EclipseAddItem(editMenu, @"Paste", @selector(paste:), @"v",
                       NSEventModifierFlagCommand, nil, nil);
        EclipseAddItem(editMenu, @"Delete", @selector(delete:), @"",
                       0, nil, nil);
        EclipseAddItem(editMenu, @"Select All", @selector(selectAll:), @"a",
                       NSEventModifierFlagCommand, nil, nil);
        [editMenu addItem:[NSMenuItem separatorItem]];
        EclipseAddItem(editMenu, @"Find…", @selector(performEclipseAction:), @"f",
                       NSEventModifierFlagCommand, gMenuTarget, @"find");
        EclipseAddItem(editMenu, @"Find Next", @selector(performEclipseAction:), @"g",
                       NSEventModifierFlagCommand, gMenuTarget, @"find_next");
        EclipseAddItem(editMenu, @"Find Previous", @selector(performEclipseAction:), @"g",
                       NSEventModifierFlagCommand | NSEventModifierFlagShift, gMenuTarget,
                       @"find_prev");

        /* --------------------------------------------------------- History */
        NSMenuItem *historyMenuItem = [[NSMenuItem alloc] initWithTitle:@"History"
                                                                 action:nil
                                                          keyEquivalent:@""];
        [mainMenu addItem:historyMenuItem];
        NSMenu *historyMenu = EclipseMenu(@"History");
        historyMenuItem.submenu = historyMenu;

        EclipseAddItem(historyMenu, @"Back", @selector(performEclipseAction:), @"[",
                       NSEventModifierFlagCommand, gMenuTarget, @"go_back");
        EclipseAddItem(historyMenu, @"Forward", @selector(performEclipseAction:), @"]",
                       NSEventModifierFlagCommand, gMenuTarget, @"go_forward");
        [historyMenu addItem:[NSMenuItem separatorItem]];
        EclipseAddItem(historyMenu, @"Home", @selector(performEclipseAction:), @"h",
                       NSEventModifierFlagCommand | NSEventModifierFlagShift, gMenuTarget, @"home");

        /* ------------------------------------------------------------ View */
        NSMenuItem *viewMenuItem = [[NSMenuItem alloc] initWithTitle:@"View"
                                                              action:nil
                                                       keyEquivalent:@""];
        [mainMenu addItem:viewMenuItem];
        NSMenu *viewMenu = EclipseMenu(@"View");
        viewMenuItem.submenu = viewMenu;

        EclipseAddItem(viewMenu, @"Reload Page", @selector(performEclipseAction:), @"r",
                       NSEventModifierFlagCommand, gMenuTarget, @"reload");
        EclipseAddItem(viewMenu, @"Stop", @selector(performEclipseAction:), @".",
                       NSEventModifierFlagCommand, gMenuTarget, @"stop_loading");
        [viewMenu addItem:[NSMenuItem separatorItem]];
        EclipseAddItem(viewMenu, @"Toggle Sidebar", @selector(performEclipseAction:), @"s",
                       NSEventModifierFlagCommand, gMenuTarget,
                       @"toggle_sidebar");
        EclipseAddItem(viewMenu, @"Toggle Sidebar (Alternate)", @selector(performEclipseAction:), @"b",
                       NSEventModifierFlagCommand | NSEventModifierFlagShift, gMenuTarget,
                       @"toggle_sidebar");
        [viewMenu addItem:[NSMenuItem separatorItem]];
        EclipseAddItem(viewMenu, @"Actual Size", @selector(performEclipseAction:), @"0",
                       NSEventModifierFlagCommand, gMenuTarget, @"zoom_reset");
        EclipseAddItem(viewMenu, @"Zoom In", @selector(performEclipseAction:), @"+",
                       NSEventModifierFlagCommand, gMenuTarget, @"zoom_in");
        EclipseAddItem(viewMenu, @"Zoom Out", @selector(performEclipseAction:), @"-",
                       NSEventModifierFlagCommand, gMenuTarget, @"zoom_out");
        [viewMenu addItem:[NSMenuItem separatorItem]];
        EclipseAddItem(viewMenu, @"Enter Full Screen", @selector(toggleFullScreen:), @"f",
                       NSEventModifierFlagCommand | NSEventModifierFlagControl, nil, nil);
        [viewMenu addItem:[NSMenuItem separatorItem]];
        EclipseAddItem(viewMenu, @"Show Web Inspector", @selector(performEclipseAction:), @"i",
                       NSEventModifierFlagCommand | NSEventModifierFlagOption, gMenuTarget,
                       @"open_devtools");

        /* ---------------------------------------------------------- Window */
        NSMenuItem *windowMenuItem = [[NSMenuItem alloc] initWithTitle:@"Window"
                                                                action:nil
                                                         keyEquivalent:@""];
        [mainMenu addItem:windowMenuItem];
        NSMenu *windowMenu = EclipseMenu(@"Window");
        windowMenuItem.submenu = windowMenu;

        EclipseAddItem(windowMenu, @"Minimize", @selector(performMiniaturize:), @"m",
                       NSEventModifierFlagCommand, nil, nil);
        EclipseAddItem(windowMenu, @"Zoom", @selector(performZoom:), @"",
                       0, nil, nil);
        [windowMenu addItem:[NSMenuItem separatorItem]];
        EclipseAddItem(windowMenu, @"Bring All to Front", @selector(arrangeInFront:), @"",
                       0, NSApp, nil);
        [NSApp setWindowsMenu:windowMenu];

        [NSApp setMainMenu:mainMenu];

        if (getenv("ECLIPSE_PERF") != NULL) {
            NSMenu *edit = [[mainMenu itemWithTitle:@"Edit"] submenu];
            fprintf(stderr,
                    "[perf] native menu installed: %lu top-level menus, edit menu has %ld items\n",
                    (unsigned long)mainMenu.numberOfItems,
                    (long)(edit != nil ? edit.numberOfItems : 0));
        }
    }
}

void eclipse_toggle_fullscreen(void) {
    NSWindow *window = [NSApp keyWindow] ?: [NSApp mainWindow];
    if (window != nil) {
        [window toggleFullScreen:nil];
    }
}

bool eclipse_is_fullscreen(void) {
    NSWindow *window = [NSApp keyWindow] ?: [NSApp mainWindow];
    if (window != nil) {
        return (window.styleMask & NSWindowStyleMaskFullScreen) != 0;
    }
    return false;
}

void eclipse_bring_view_to_front(void *viewPtr) {
    if (viewPtr == NULL) return;
    @autoreleasepool {
        NSView *view = (__bridge NSView *)viewPtr;
        NSView *superview = [view superview];
        if (superview != nil) {
            [superview addSubview:view positioned:NSWindowAbove relativeTo:nil];
        }
    }
}

void eclipse_configure_window(void *windowPtr) {
    if (windowPtr == NULL) return;
    @autoreleasepool {
        NSWindow *window = (__bridge NSWindow *)windowPtr;
        [window setMovableByWindowBackground:NO];
        [window setAcceptsMouseMovedEvents:YES];
        window.backgroundColor = [NSColor colorWithCalibratedRed:0.063 green:0.071 blue:0.094 alpha:1.0];

        NSView *contentView = [window contentView];
        if (contentView != nil) {
            NSTrackingArea *trackingArea = [[NSTrackingArea alloc]
                initWithRect:NSZeroRect
                options:NSTrackingMouseMoved | NSTrackingMouseEnteredAndExited | NSTrackingActiveAlways | NSTrackingInVisibleRect
                owner:contentView
                userInfo:nil];
            [contentView addTrackingArea:trackingArea];
        }
    }
}

void eclipse_enable_swipe_gestures(void *viewPtr) {
    if (viewPtr == NULL) return;
    @autoreleasepool {
        NSView *view = (__bridge NSView *)viewPtr;
        NSMutableArray *queue = [NSMutableArray arrayWithObject:view];
        while (queue.count > 0) {
            NSView *v = [queue firstObject];
            [queue removeObjectAtIndex:0];
            if ([v respondsToSelector:@selector(setAllowsBackForwardNavigationGestures:)]) {
                [(id)v setAllowsBackForwardNavigationGestures:YES];
            }
            if ([v respondsToSelector:@selector(setAllowsMagnification:)]) {
                [(id)v setAllowsMagnification:YES];
            }
            [queue addObjectsFromArray:v.subviews];
        }
    }
}

void eclipse_style_webview_card(void *viewPtr) {
    if (viewPtr == NULL) return;
    @autoreleasepool {
        NSView *view = (__bridge NSView *)viewPtr;
        view.wantsLayer = YES;
        view.layer.cornerRadius = 10.0;
        view.layer.masksToBounds = YES;
        view.layer.borderWidth = 1.0;
        view.layer.borderColor = [NSColor colorWithCalibratedWhite:1.0 alpha:0.12].CGColor;
        eclipse_enable_swipe_gestures(viewPtr);
    }
}

void eclipse_animate_view_frame(void *viewPtr, double x, double y, double width, double height, double duration) {
    if (viewPtr == NULL) return;
    @autoreleasepool {
        NSView *view = (__bridge NSView *)viewPtr;
        NSView *superview = [view superview];
        if (superview == nil) return;
        CGFloat actualY = y;
        if (![superview isFlipped]) {
            actualY = superview.frame.size.height - y - height;
        }
        NSRect targetFrame = NSMakeRect(x, actualY, width, height);

        if (duration <= 0.001) {
            [view setFrame:targetFrame];
            return;
        }

        [NSAnimationContext runAnimationGroup:^(NSAnimationContext *ctx) {
            ctx.duration = duration;
            ctx.timingFunction = [CAMediaTimingFunction functionWithControlPoints:(float)0.16 :(float)1.0 :(float)0.3 :(float)1.0];
            ctx.allowsImplicitAnimation = YES;
            [[view animator] setFrame:targetFrame];
        }];
    }
}

static double gSidebarWidth = 240.0;
static BOOL gIsHoverExpanded = NO;
static void (*gEdgeExpandCallback)(void) = NULL;
static void (*gEdgeCollapseCallback)(void) = NULL;
static id gLocalMouseMonitor = nil;
static NSTimer *gMousePollTimer = nil;

static void checkMousePosition(NSPoint screenPt) {
    if (gEdgeExpandCallback == NULL || gEdgeCollapseCallback == NULL) return;
    NSWindow *window = [NSApp keyWindow] ?: [NSApp mainWindow];
    if (window == nil || ![window isKeyWindow]) return;
    if ((window.styleMask & NSWindowStyleMaskFullScreen) != 0) return;

    NSRect windowFrame = [window frame];
    if (!NSPointInRect(screenPt, windowFrame)) {
        if (gIsHoverExpanded) {
            gEdgeCollapseCallback();
        }
        return;
    }

    CGFloat winX = screenPt.x - windowFrame.origin.x;
    if (!gIsHoverExpanded) {
        if (winX >= 0.0 && winX <= 14.0) {
            gEdgeExpandCallback();
        }
    } else {
        if (winX > (gSidebarWidth + 20.0) || winX < -15.0) {
            gEdgeCollapseCallback();
        }
    }
}

void eclipse_set_hover_expanded(int is_expanded, double sidebar_width) {
    gIsHoverExpanded = is_expanded ? YES : NO;
    if (sidebar_width > 50.0) {
        gSidebarWidth = sidebar_width;
    }
}

void eclipse_install_edge_monitor(void (*expand_cb)(void), void (*collapse_cb)(void)) {
    gEdgeExpandCallback = expand_cb;
    gEdgeCollapseCallback = collapse_cb;

    if (gLocalMouseMonitor == nil) {
        gLocalMouseMonitor = [NSEvent addLocalMonitorForEventsMatchingMask:NSEventMaskMouseMoved | NSEventMaskLeftMouseDragged
                                                                   handler:^NSEvent *(NSEvent *event) {
            NSWindow *window = [event window];
            if (window != nil && [window isKeyWindow] && ((window.styleMask & NSWindowStyleMaskFullScreen) == 0)) {
                NSPoint pt = [event locationInWindow];
                if (!gIsHoverExpanded) {
                    if (pt.x >= 0.0 && pt.x <= 14.0) {
                        if (gEdgeExpandCallback) gEdgeExpandCallback();
                    }
                } else {
                    if (pt.x > (gSidebarWidth + 20.0) || pt.x < -15.0) {
                        if (gEdgeCollapseCallback) gEdgeCollapseCallback();
                    }
                }
            }
            return event;
        }];
    }

    if (gMousePollTimer == nil) {
        gMousePollTimer = [NSTimer scheduledTimerWithTimeInterval:0.06
                                                          repeats:YES
                                                            block:^(NSTimer * _Nonnull timer) {
            (void)timer;
            checkMousePosition([NSEvent mouseLocation]);
        }];
        [[NSRunLoop mainRunLoop] addTimer:gMousePollTimer forMode:NSRunLoopCommonModes];
    }
}
