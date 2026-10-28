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

void eclipse_compile_content_rule_list_if_needed(void);

void eclipse_install_menu(EclipseMenuCallback callback) {
    @autoreleasepool {
        [NSApplication sharedApplication];
        gCallback = callback;
        gMenuTarget = [[EclipseMenuTarget alloc] init];
        eclipse_compile_content_rule_list_if_needed();

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
        EclipseAddItem(fileMenu, @"New Little Arc Window", @selector(performEclipseAction:), @"n",
                       NSEventModifierFlagCommand | NSEventModifierFlagOption, gMenuTarget, @"little_arc");
        EclipseAddItem(fileMenu, @"New Note", @selector(performEclipseAction:), @"n",
                       NSEventModifierFlagCommand | NSEventModifierFlagShift, gMenuTarget, @"open_notes");
        EclipseAddItem(fileMenu, @"New Easel", @selector(performEclipseAction:), @"e",
                       NSEventModifierFlagCommand | NSEventModifierFlagShift, gMenuTarget, @"open_easel");
        EclipseAddItem(fileMenu, @"Tidy Tabs", @selector(performEclipseAction:), @"y",
                       NSEventModifierFlagCommand | NSEventModifierFlagShift, gMenuTarget, @"tidy_tabs");
        EclipseAddItem(fileMenu, @"Close Tab", @selector(performEclipseAction:), @"w",
                       NSEventModifierFlagCommand, gMenuTarget, @"close_tab");
        EclipseAddItem(fileMenu, @"Reopen Closed Tab", @selector(performEclipseAction:), @"t",
                       NSEventModifierFlagCommand | NSEventModifierFlagShift, gMenuTarget,
                       @"reopen_closed_tab");
        [fileMenu addItem:[NSMenuItem separatorItem]];
        EclipseAddItem(fileMenu, @"Command Bar…", @selector(performEclipseAction:), @"k",
                       NSEventModifierFlagCommand, gMenuTarget, @"command_bar");
        EclipseAddItem(fileMenu, @"Open Location…", @selector(performEclipseAction:), @"l",
                       NSEventModifierFlagCommand, gMenuTarget, @"focus_omnibar");
        EclipseAddItem(fileMenu, @"Downloads", @selector(performEclipseAction:), @"j",
                       NSEventModifierFlagCommand, gMenuTarget, @"open_downloads");
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
            NSArray *subviews = [superview subviews];
            if ([subviews count] > 0 && [subviews lastObject] == view) {
                return;
            }
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
            contentView.wantsLayer = YES;
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

/* ------------------------------------------------------------- Real Native Ad & Tracker Blocker */
static WKContentRuleList *gEclipseContentRuleList = nil;
static BOOL gIsCompilingRuleList = NO;
static NSHashTable *gRegisteredWebViews = nil;

static WKWebView *eclipse_find_wkwebview(NSView *root) {
    if (root == nil) return nil;
    if ([root isKindOfClass:[WKWebView class]]) {
        return (WKWebView *)root;
    }
    NSMutableArray *queue = [NSMutableArray arrayWithObject:root];
    while (queue.count > 0) {
        NSView *v = [queue firstObject];
        [queue removeObjectAtIndex:0];
        if ([v isKindOfClass:[WKWebView class]]) {
            return (WKWebView *)v;
        }
        [queue addObjectsFromArray:v.subviews];
    }
    return nil;
}

static void eclipse_attach_rule_list(WKWebView *wv) {
    if (wv == nil || gEclipseContentRuleList == nil) return;
    @try {
        [wv.configuration.userContentController removeContentRuleList:gEclipseContentRuleList];
    } @catch (id ex) {}
    @try {
        [wv.configuration.userContentController addContentRuleList:gEclipseContentRuleList];
    } @catch (id ex) {}
}

static void eclipse_detach_rule_list(WKWebView *wv) {
    if (wv == nil || gEclipseContentRuleList == nil) return;
    @try {
        [wv.configuration.userContentController removeContentRuleList:gEclipseContentRuleList];
    } @catch (id ex) {}
}

void eclipse_compile_content_rule_list_if_needed(void) {
    if (gEclipseContentRuleList != nil || gIsCompilingRuleList) return;
    gIsCompilingRuleList = YES;

    WKContentRuleListStore *store = [WKContentRuleListStore defaultStore];
    NSString *json = @"["
        "  {"
        "    \"trigger\": {"
        "      \"url-filter\": \"^https?://([a-z0-9.-]+\\\\.)?(doubleclick\\\\.net|googlesyndication\\\\.com|googleadservices\\\\.com|adservice\\\\.google\\\\.com|2mdn\\\\.net|adnxs\\\\.com|criteo\\\\.com|criteo\\\\.net|rubiconproject\\\\.com|pubmatic\\\\.com|openx\\\\.net|casalemedia\\\\.com|smartadserver\\\\.com)[/:?#]\""
        "    },"
        "    \"action\": { \"type\": \"block\" }"
        "  },"
        "  {"
        "    \"trigger\": {"
        "      \"url-filter\": \"^https?://([a-z0-9.-]+\\\\.)?(advertising\\\\.com|bidswitch\\\\.net|contextweb\\\\.com|serving-sys\\\\.com|flashtalking\\\\.com|sovrn\\\\.com|lijit\\\\.com|spotxchange\\\\.com|springserve\\\\.com|triplelift\\\\.com|teads\\\\.tv|inmobi\\\\.com|adtech\\\\.de|media\\\\.net|amazon-adsystem\\\\.com)[/:?#]\""
        "    },"
        "    \"action\": { \"type\": \"block\" }"
        "  },"
        "  {"
        "    \"trigger\": {"
        "      \"url-filter\": \"^https?://([a-z0-9.-]+\\\\.)?(adroll\\\\.com|quantserve\\\\.com|scorecardresearch\\\\.com|outbrain\\\\.com|taboola\\\\.com|revcontent\\\\.com|mgid\\\\.com|zemanta\\\\.com|zergnet\\\\.com|moatads\\\\.com|iasds01\\\\.com|adsafeprotected\\\\.com|yieldmo\\\\.com|sharethrough\\\\.com)[/:?#]\""
        "    },"
        "    \"action\": { \"type\": \"block\" }"
        "  },"
        "  {"
        "    \"trigger\": {"
        "      \"url-filter\": \"^https?://([a-z0-9.-]+\\\\.)?(popads\\\\.net|popcash\\\\.net|propellerads\\\\.com|exoclick\\\\.com|trafficjunky\\\\.com|juicyads\\\\.com|ad-delivery\\\\.net|exponential\\\\.com|undertone\\\\.com|conversantmedia\\\\.com|unrulymedia\\\\.com|adblade\\\\.com|nativeads\\\\.com|adsterra\\\\.com)[/:?#]\""
        "    },"
        "    \"action\": { \"type\": \"block\" }"
        "  },"
        "  {"
        "    \"trigger\": {"
        "      \"url-filter\": \"^https?://([a-z0-9.-]+\\\\.)?(buysellads\\\\.com|mediavine\\\\.com|adthrive\\\\.com|shemedia\\\\.com|carbonads\\\\.net|google-analytics\\\\.com|googletagmanager\\\\.com|hotjar\\\\.com|clarity\\\\.ms|mouseflow\\\\.com|fullstory\\\\.com|segment\\\\.io|mixpanel\\\\.com|amplitude\\\\.com|statcounter\\\\.com|crazyegg\\\\.com)[/:?#]\","
        "      \"load-type\": [\"third-party\"]"
        "    },"
        "    \"action\": { \"type\": \"block\" }"
        "  },"
        "  {"
        "    \"trigger\": { \"url-filter\": \"^https?://([a-z0-9.-]+\\\\.)?youtube\\\\.com/(youtubei/v1/player/ad_break|pagead/|api/stats/ads|ptracking|get_midroll_info)\" },"
        "    \"action\": { \"type\": \"block\" }"
        "  },"
        "  {"
        "    \"trigger\": { \"url-filter\": \"^https?://\" },"
        "    \"action\": {"
        "      \"type\": \"css-display-none\","
        "      \"selector\": \".adsbygoogle, [id^=google_ads_], .ad-banner, .ad-unit, .advertisement, .ad-container, [data-ad-slot], .ytp-ad-overlay-container, .ytp-ad-message-container, ytd-ad-slot-renderer, ytd-in-feed-ad-layout-renderer, ytd-banner-promo-renderer, ytd-statement-banner-renderer, #masthead-ad, #player-ads, .sparkles-light-cta, ytd-promoted-sparkles-web-renderer, ytd-display-ad-renderer, ytd-promoted-video-renderer, ytd-compact-promoted-video-renderer, ytd-ads-engagement-panel-content-renderer, ytd-enforcement-message-view-model, .trc_related_container, .outbrain, .taboola, .ytp-ad-player-overlay, .ytp-ad-progress\""
        "    }"
        "  }"
        "]";

    [store compileContentRuleListForIdentifier:@"eclipse-native-shields-v3"
                       encodedContentRuleList:json
                            completionHandler:^(WKContentRuleList *list, NSError *error) {
        gIsCompilingRuleList = NO;
        if (error != nil || list == nil) {
            return;
        }
        gEclipseContentRuleList = list;
        if (gRegisteredWebViews != nil) {
            for (WKWebView *wv in [gRegisteredWebViews allObjects]) {
                eclipse_attach_rule_list(wv);
            }
        }
    }];
}

void eclipse_install_native_adblocker(void *viewPtr, int enabled) {
    if (viewPtr == NULL) return;
    @autoreleasepool {
        static dispatch_once_t onceToken;
        dispatch_once(&onceToken, ^{
            gRegisteredWebViews = [NSHashTable weakObjectsHashTable];
        });

        NSView *view = (__bridge NSView *)viewPtr;
        WKWebView *wv = eclipse_find_wkwebview(view);
        if (wv == nil) return;

        if (enabled) {
            [gRegisteredWebViews addObject:wv];
            if (gEclipseContentRuleList != nil) {
                eclipse_attach_rule_list(wv);
            } else {
                eclipse_compile_content_rule_list_if_needed();
            }
        } else {
            [gRegisteredWebViews removeObject:wv];
            eclipse_detach_rule_list(wv);
        }
    }
}

void eclipse_set_native_adblocker_enabled(void *viewPtr, int enabled) {
    eclipse_install_native_adblocker(viewPtr, enabled);
}

void eclipse_style_webview_card(void *viewPtr) {
    if (viewPtr == NULL) return;
    @autoreleasepool {
        NSView *view = (__bridge NSView *)viewPtr;
        view.wantsLayer = YES;
        view.layerContentsRedrawPolicy = NSViewLayerContentsRedrawOnSetNeedsDisplay;
        view.layer.cornerRadius = 12.0;
        if (@available(macOS 10.15, *)) {
            view.layer.cornerCurve = kCACornerCurveContinuous;
        }
        view.layer.masksToBounds = YES;
        view.layer.borderWidth = 1.0;
        view.layer.borderColor = [NSColor colorWithCalibratedWhite:1.0 alpha:0.13].CGColor;
        eclipse_enable_swipe_gestures(viewPtr);
    }
}

void eclipse_style_sidebar_view(void *viewPtr) {
    if (viewPtr == NULL) return;
    @autoreleasepool {
        NSView *view = (__bridge NSView *)viewPtr;
        view.wantsLayer = YES;
        view.layerContentsRedrawPolicy = NSViewLayerContentsRedrawOnSetNeedsDisplay;
        view.layer.cornerRadius = 12.0;
        if (@available(macOS 10.15, *)) {
            view.layer.cornerCurve = kCACornerCurveContinuous;
        }
        view.layer.masksToBounds = YES;
    }
}

void eclipse_animate_view_frame(void *viewPtr, double x, double y, double width, double height, double duration) {
    if (viewPtr == NULL) return;
    @autoreleasepool {
        NSView *view = (__bridge NSView *)viewPtr;
        NSView *superview = [view superview];
        if (superview == nil) return;
        view.wantsLayer = YES;

        CGFloat actualY = y;
        if (![superview isFlipped]) {
            actualY = superview.frame.size.height - y - height;
        }
        NSRect targetFrame = NSMakeRect(x, actualY, width, height);
        NSRect currentFrame = view.frame;

        if (duration <= 0.001) {
            [CATransaction begin];
            [CATransaction setDisableActions:YES];
            [view.layer removeAllAnimations];
            [view setFrame:targetFrame];
            [CATransaction commit];
            return;
        }

        BOOL sizeChanged = (fabs(currentFrame.size.width - width) > 0.5 ||
                            fabs(currentFrame.size.height - height) > 0.5);

        CAMediaTimingFunction *springCurve =
            [CAMediaTimingFunction functionWithControlPoints:(float)0.22 :(float)1.0 :(float)0.36 :(float)1.0];

        if (!sizeChanged) {
            [NSAnimationContext runAnimationGroup:^(NSAnimationContext *ctx) {
                ctx.duration = duration;
                ctx.timingFunction = springCurve;
                ctx.allowsImplicitAnimation = YES;
                [[view animator] setFrameOrigin:targetFrame.origin];
            }];
        } else {
            CALayer *layer = view.layer;
            CALayer *pres = [layer presentationLayer] ?: layer;
            CGPoint oldPos = pres.position;
            CGRect oldBounds = pres.bounds;

            [CATransaction begin];
            [CATransaction setDisableActions:YES];
            [view setFrame:targetFrame];
            [CATransaction commit];

            if (layer != nil) {
                CABasicAnimation *posAnim = [CABasicAnimation animationWithKeyPath:@"position"];
                posAnim.fromValue = [NSValue valueWithPoint:NSPointFromCGPoint(oldPos)];
                posAnim.toValue = [NSValue valueWithPoint:NSPointFromCGPoint(layer.position)];
                posAnim.duration = duration;
                posAnim.timingFunction = springCurve;

                CABasicAnimation *boundsAnim = [CABasicAnimation animationWithKeyPath:@"bounds"];
                boundsAnim.fromValue = [NSValue valueWithRect:NSRectFromCGRect(oldBounds)];
                boundsAnim.toValue = [NSValue valueWithRect:NSRectFromCGRect(layer.bounds)];
                boundsAnim.duration = duration;
                boundsAnim.timingFunction = springCurve;

                [layer addAnimation:posAnim forKey:@"eclipsePositionAnim"];
                [layer addAnimation:boundsAnim forKey:@"eclipseBoundsAnim"];
            }
        }
    }
}

static double gSidebarWidth = 232.0;
static BOOL gIsHoverExpanded = NO;
static CFAbsoluteTime gOutsideSince = 0.0;
static void (*gEdgeExpandCallback)(void) = NULL;
static void (*gEdgeCollapseCallback)(void) = NULL;
static id gLocalMouseMonitor = nil;
static NSTimer *gMousePollTimer = nil;

static void evaluateEdgeHoverAtWindowPoint(NSPoint pt, CGFloat winH) {
    if (gEdgeExpandCallback == NULL || gEdgeCollapseCallback == NULL) return;

    if (!gIsHoverExpanded) {
        gOutsideSince = 0.0;
        if (pt.x >= -2.0 && pt.x <= 12.0 && pt.y >= 0.0 && pt.y <= winH) {
            gIsHoverExpanded = YES;
            gEdgeExpandCallback();
        }
    } else {
        BOOL isOutside = (pt.x > (gSidebarWidth + 24.0) || pt.x < -20.0 || pt.y < -10.0 || pt.y > (winH + 10.0));
        if (!isOutside) {
            gOutsideSince = 0.0;
        } else {
            CFAbsoluteTime now = CFAbsoluteTimeGetCurrent();
            if (gOutsideSince <= 0.0) {
                gOutsideSince = now;
            } else if ((now - gOutsideSince) >= 0.075) {
                gIsHoverExpanded = NO;
                gOutsideSince = 0.0;
                gEdgeCollapseCallback();
            }
        }
    }
}

static void checkMousePosition(NSPoint screenPt) {
    if (gEdgeExpandCallback == NULL || gEdgeCollapseCallback == NULL) return;
    NSWindow *window = [NSApp keyWindow] ?: [NSApp mainWindow];
    if (window == nil || ![window isKeyWindow]) return;
    if ((window.styleMask & NSWindowStyleMaskFullScreen) != 0) return;

    NSRect windowFrame = [window frame];
    NSPoint pt = NSMakePoint(screenPt.x - windowFrame.origin.x, screenPt.y - windowFrame.origin.y);
    evaluateEdgeHoverAtWindowPoint(pt, windowFrame.size.height);
}

void eclipse_set_hover_expanded(int is_expanded, double sidebar_width) {
    gIsHoverExpanded = is_expanded ? YES : NO;
    gOutsideSince = 0.0;
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
                evaluateEdgeHoverAtWindowPoint([event locationInWindow], window.frame.size.height);
            }
            return event;
        }];
    }

    if (gMousePollTimer == nil) {
        gMousePollTimer = [NSTimer scheduledTimerWithTimeInterval:0.032
                                                          repeats:YES
                                                            block:^(NSTimer * _Nonnull timer) {
            (void)timer;
            checkMousePosition([NSEvent mouseLocation]);
        }];
        [[NSRunLoop mainRunLoop] addTimer:gMousePollTimer forMode:NSRunLoopCommonModes];
    }
}

/* ---- v1.0 scaffold: implemented in the fix phase ------------------------ */

typedef void (*EclipseNavigationCallback)(uint64_t tabId, int kind, int64_t code,
                                          const char *description, const char *url);

void eclipse_install_navigation_hooks(EclipseNavigationCallback callback) {
    (void)callback;
}

void eclipse_register_tab_view(void *viewPtr, uint64_t tabId) {
    (void)viewPtr;
    (void)tabId;
}

void eclipse_recheck_hover(void) {}
