#import <Cocoa/Cocoa.h>
#import <QuartzCore/QuartzCore.h>
#import <WebKit/WebKit.h>
#import <objc/message.h>
#import <objc/runtime.h>
#include <stdlib.h>
#include <stdio.h>

typedef void (*EclipseMenuCallback)(const char *action);

@class EclipseMenuTarget;

static EclipseMenuCallback gCallback = NULL;
static EclipseMenuTarget *gMenuTarget = nil;
static NSMenuItem *gFullScreenMenuItem = nil;

/* The browser window, set by eclipse_configure_window. Fullscreen and edge
   hover always act on it, never on whatever window happens to be key (the Web
   Inspector, the About panel or WebKit's element-fullscreen window). */
static __weak NSWindow *gBrowserWindow = nil;

static NSWindow *eclipse_browser_window(void) {
    NSWindow *window = gBrowserWindow;
    return window != nil ? window : [NSApp mainWindow];
}

static BOOL eclipse_is_little_window(NSWindow *window);
static BOOL eclipse_little_window_perform_action(NSWindow *window, NSString *action);
static void eclipse_end_all_sheets(NSWindow *window);

/* The window a shortcut is meant for: the key window, or the window a sheet
   (a JavaScript dialog, a print or open panel) is attached to while that
   sheet is key. */
static NSWindow *eclipse_shortcut_window(void) {
    NSWindow *window = [NSApp keyWindow];
    while (window.sheetParent != nil) window = window.sheetParent;
    return window;
}

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

    NSWindow *key = eclipse_shortcut_window();
    NSWindow *browser = gBrowserWindow;

    // Page commands act on a Little Eclipse window when it is key (sign-in
    // popups, see below), never on the hidden tab behind it.
    if (eclipse_little_window_perform_action(key, action)) return;

    // ⌘W / ⇧⌘W in a secondary window (detached Web Inspector, About panel,
    // a Little Eclipse window) closes that window instead of a browser tab behind it.
    if ([action isEqualToString:@"close_tab"] || [action isEqualToString:@"close_window"]) {
        if (key != nil && browser != nil && key != browser &&
            (key.styleMask & NSWindowStyleMaskClosable) != 0) {
            // AppKit does not close a window while a sheet is attached; a
            // popup's dialog is answered (Cancel) first.
            if (eclipse_is_little_window(key)) eclipse_end_all_sheets(key);
            [key performClose:nil];
            return;
        }
    }
    // Everything else is about the browser window (tabs, sidebar, Settings):
    // it comes to the front instead of changing out of sight behind a popup.
    if (browser != nil && key != browser && eclipse_is_little_window(key)) {
        [browser makeKeyAndOrderFront:nil];
    }
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

/* A menu item that is never shown but whose key equivalent still works, for
   alternate shortcuts of a visible item (⌘= for Zoom In, ⌃Tab, ...). */
static NSMenuItem *EclipseAddHiddenItem(NSMenu *menu,
                                        NSString *title,
                                        NSString *key,
                                        NSEventModifierFlags modifiers,
                                        NSString *identifier) {
    NSMenuItem *item = EclipseAddItem(menu, title, @selector(performEclipseAction:), key,
                                      modifiers, gMenuTarget, identifier);
    item.hidden = YES;
    if (@available(macOS 10.13, *)) {
        item.allowsKeyEquivalentWhenHidden = YES;
    }
    return item;
}

static NSMenu *EclipseMenu(NSString *title) {
    return [[NSMenu alloc] initWithTitle:title];
}

static void eclipse_compile_content_rule_list_if_needed(void);

void eclipse_install_menu(EclipseMenuCallback callback) {
    @autoreleasepool {
        [NSApplication sharedApplication];
        // Eclipse draws its own tabs; AppKit's native window tabs would add
        // a tab bar over the custom chrome and "Merge All Windows" items.
        if ([NSWindow respondsToSelector:@selector(setAllowsAutomaticWindowTabbing:)]) {
            [NSWindow setAllowsAutomaticWindowTabbing:NO];
        }
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
        // Routed through Rust so open internal pages can save pending edits
        // before the app exits.
        EclipseAddItem(appMenu, @"Quit Eclipse", @selector(performEclipseAction:), @"q",
                       NSEventModifierFlagCommand, gMenuTarget, @"quit");

        /* ------------------------------------------------------------ File */
        NSMenuItem *fileMenuItem = [[NSMenuItem alloc] initWithTitle:@"File"
                                                              action:nil
                                                       keyEquivalent:@""];
        [mainMenu addItem:fileMenuItem];
        NSMenu *fileMenu = EclipseMenu(@"File");
        fileMenuItem.submenu = fileMenu;

        EclipseAddItem(fileMenu, @"New Tab", @selector(performEclipseAction:), @"t",
                       NSEventModifierFlagCommand, gMenuTarget, @"new_tab");
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
        EclipseAddHiddenItem(viewMenu, @"Zoom In", @"=", NSEventModifierFlagCommand, @"zoom_in");
        EclipseAddItem(viewMenu, @"Zoom Out", @selector(performEclipseAction:), @"-",
                       NSEventModifierFlagCommand, gMenuTarget, @"zoom_out");
        [viewMenu addItem:[NSMenuItem separatorItem]];
        // Targets the browser window once eclipse_configure_window runs, so
        // the Web Inspector or another key window is never the one toggled.
        // NSWindow validates the item and swaps in "Exit Full Screen".
        gFullScreenMenuItem = EclipseAddItem(viewMenu, @"Enter Full Screen", @selector(toggleFullScreen:), @"f",
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

        // Tab navigation. "{" / "}" with ⌘ is how AppKit matches ⇧⌘[ / ⇧⌘]
        // (a "[" key equivalent with a Shift mask would also swallow ⌘[ Back),
        // and ⌃⇧Tab arrives as the back-tab character.
        EclipseAddItem(windowMenu, @"Show Previous Tab", @selector(performEclipseAction:), @"{",
                       NSEventModifierFlagCommand, gMenuTarget, @"prev_tab");
        EclipseAddItem(windowMenu, @"Show Next Tab", @selector(performEclipseAction:), @"}",
                       NSEventModifierFlagCommand, gMenuTarget, @"next_tab");
        EclipseAddHiddenItem(windowMenu, @"Show Next Tab", @"\t",
                             NSEventModifierFlagControl, @"next_tab");
        EclipseAddHiddenItem(windowMenu, @"Show Previous Tab",
                             [NSString stringWithFormat:@"%C", (unichar)NSBackTabCharacter],
                             NSEventModifierFlagControl | NSEventModifierFlagShift, @"prev_tab");

        NSMenuItem *selectTabItem = [[NSMenuItem alloc] initWithTitle:@"Select Tab"
                                                               action:nil
                                                        keyEquivalent:@""];
        NSMenu *selectTabMenu = EclipseMenu(@"Select Tab");
        selectTabItem.submenu = selectTabMenu;
        [windowMenu addItem:selectTabItem];
        for (int i = 0; i < 8; i++) {
            EclipseAddItem(selectTabMenu, [NSString stringWithFormat:@"Tab %d", i + 1],
                           @selector(performEclipseAction:), [NSString stringWithFormat:@"%d", i + 1],
                           NSEventModifierFlagCommand, gMenuTarget,
                           [NSString stringWithFormat:@"select_tab_index:%d", i]);
        }
        EclipseAddItem(selectTabMenu, @"Last Tab", @selector(performEclipseAction:), @"9",
                       NSEventModifierFlagCommand, gMenuTarget, @"select_last_tab");
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
    NSWindow *window = eclipse_browser_window();
    if (window != nil) {
        [window toggleFullScreen:nil];
    }
}

bool eclipse_is_fullscreen(void) {
    NSWindow *window = eclipse_browser_window();
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
        gBrowserWindow = window;
        gFullScreenMenuItem.target = window;
        [window setMovableByWindowBackground:NO];
        [window setAcceptsMouseMovedEvents:YES];
        window.backgroundColor = [NSColor colorWithCalibratedRed:0.063 green:0.071 blue:0.094 alpha:1.0];
        // Everything sits on this solid color, so the window needs no
        // per-pixel transparency; an opaque window lets WindowServer skip
        // blending it and cull what is underneath.
        [window setOpaque:YES];
        if ([window respondsToSelector:@selector(setTabbingMode:)]) {
            window.tabbingMode = NSWindowTabbingModeDisallowed;
        }

        NSView *contentView = [window contentView];
        if (contentView != nil) {
            contentView.wantsLayer = YES;
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

/* ------------------------------------------------------------- Real Native Ad & Tracker Blocker */
/* WebKit content-blocker regexes have no '|', so every domain gets its own
   rule. Ad and tracker hosts are blocked only as third-party loads, which
   keeps direct visits to those companies' own sites working. */
static WKContentRuleList *gEclipseContentRuleList = nil;
static BOOL gIsLoadingRuleList = NO;
static BOOL gRuleListFailed = NO;
static NSHashTable<WKWebView *> *gAdblockWantedWebViews = nil;
static NSHashTable<WKWebView *> *gAdblockAttachedWebViews = nil;
static NSString *const kEclipseRuleListPrefix = @"eclipse-native-shields-";

static NSArray<NSString *> *eclipse_blocked_third_party_domains(void) {
    return @[
        // Ad networks and exchanges.
        @"doubleclick.net", @"googlesyndication.com", @"googleadservices.com",
        @"adservice.google.com", @"2mdn.net", @"adnxs.com", @"criteo.com", @"criteo.net",
        @"rubiconproject.com", @"pubmatic.com", @"openx.net", @"casalemedia.com",
        @"smartadserver.com", @"advertising.com", @"bidswitch.net", @"contextweb.com",
        @"serving-sys.com", @"flashtalking.com", @"sovrn.com", @"lijit.com",
        @"spotxchange.com", @"springserve.com", @"triplelift.com", @"teads.tv",
        @"inmobi.com", @"adtech.de", @"media.net", @"amazon-adsystem.com",
        @"adroll.com", @"quantserve.com", @"scorecardresearch.com", @"outbrain.com",
        @"taboola.com", @"revcontent.com", @"mgid.com", @"zemanta.com", @"zergnet.com",
        @"moatads.com", @"iasds01.com", @"adsafeprotected.com", @"yieldmo.com",
        @"sharethrough.com", @"popads.net", @"popcash.net", @"propellerads.com",
        @"exoclick.com", @"trafficjunky.com", @"juicyads.com", @"ad-delivery.net",
        @"exponential.com", @"undertone.com", @"conversantmedia.com", @"unrulymedia.com",
        @"adblade.com", @"nativeads.com", @"adsterra.com", @"buysellads.com",
        @"mediavine.com", @"adthrive.com", @"shemedia.com", @"carbonads.net",
        // Analytics and session-recording trackers.
        @"google-analytics.com", @"googletagmanager.com", @"hotjar.com", @"clarity.ms",
        @"mouseflow.com", @"fullstory.com", @"segment.io", @"mixpanel.com",
        @"amplitude.com", @"statcounter.com", @"crazyegg.com",
    ];
}

static NSString *eclipse_regex_escape_domain(NSString *domain) {
    return [domain stringByReplacingOccurrencesOfString:@"." withString:@"\\."];
}

static NSString *eclipse_native_shields_json(void) {
    NSMutableArray *rules = [NSMutableArray array];
    for (NSString *domain in eclipse_blocked_third_party_domains()) {
        NSString *filter = [NSString stringWithFormat:@"^https?://([^/]*\\.)?%@[/:?#]",
                                                      eclipse_regex_escape_domain(domain)];
        [rules addObject:@{
            @"trigger": @{ @"url-filter": filter, @"load-type": @[ @"third-party" ] },
            @"action": @{ @"type": @"block" },
        }];
    }

    // YouTube serves its ads first-party, so these paths are blocked on any load.
    for (NSString *path in @[ @"youtubei/v1/player/ad_break", @"pagead/", @"api/stats/ads",
                              @"ptracking", @"get_midroll_info" ]) {
        NSString *filter = [NSString stringWithFormat:@"^https?://([^/]*\\.)?youtube\\.com/%@", path];
        [rules addObject:@{
            @"trigger": @{ @"url-filter": filter },
            @"action": @{ @"type": @"block" },
        }];
    }

    // Cosmetic rules: only selectors specific to ad products everywhere, and
    // YouTube's ad containers on YouTube alone.
    [rules addObject:@{
        @"trigger": @{ @"url-filter": @"^https?://" },
        @"action": @{
            @"type": @"css-display-none",
            @"selector": @".adsbygoogle, [id^=\"google_ads_\"], .trc_related_container, .outbrain, .taboola",
        },
    }];
    [rules addObject:@{
        @"trigger": @{ @"url-filter": @".*", @"if-domain": @[ @"*youtube.com" ] },
        @"action": @{
            @"type": @"css-display-none",
            @"selector": @".ytp-ad-overlay-container, .ytp-ad-message-container, ytd-ad-slot-renderer, "
                         @"ytd-in-feed-ad-layout-renderer, ytd-banner-promo-renderer, "
                         @"ytd-statement-banner-renderer, #masthead-ad, #player-ads, .sparkles-light-cta, "
                         @"ytd-promoted-sparkles-web-renderer, ytd-display-ad-renderer, "
                         @"ytd-promoted-video-renderer, ytd-compact-promoted-video-renderer, "
                         @"ytd-ads-engagement-panel-content-renderer, ytd-enforcement-message-view-model, "
                         @".ytp-ad-player-overlay, .ytp-ad-progress",
        },
    }];

    NSData *data = [NSJSONSerialization dataWithJSONObject:rules
                                                   options:NSJSONWritingSortedKeys
                                                     error:nil];
    return data != nil ? [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding] : @"[]";
}

/* Stable FNV-1a hash of the rule JSON, so the compiled list's identifier
   changes whenever the rules do and a cached list is reused otherwise. */
static NSString *eclipse_rule_list_identifier(NSString *json) {
    uint64_t hash = 0xcbf29ce484222325ULL;
    const unsigned char *bytes = (const unsigned char *)json.UTF8String;
    for (; bytes != NULL && *bytes != 0; bytes++) {
        hash ^= *bytes;
        hash *= 0x100000001b3ULL;
    }
    return [NSString stringWithFormat:@"%@%016llx", kEclipseRuleListPrefix, (unsigned long long)hash];
}

static void eclipse_attach_rule_list(WKWebView *wv) {
    if (wv == nil || gEclipseContentRuleList == nil) return;
    if ([gAdblockAttachedWebViews containsObject:wv]) return;
    @try {
        [wv.configuration.userContentController addContentRuleList:gEclipseContentRuleList];
        [gAdblockAttachedWebViews addObject:wv];
    } @catch (id ex) {}
}

static void eclipse_detach_rule_list(WKWebView *wv) {
    if (wv == nil || gEclipseContentRuleList == nil) return;
    if (![gAdblockAttachedWebViews containsObject:wv]) return;
    @try {
        [wv.configuration.userContentController removeContentRuleList:gEclipseContentRuleList];
    } @catch (id ex) {}
    [gAdblockAttachedWebViews removeObject:wv];
}

static void eclipse_rule_list_ready(WKContentRuleList *list) {
    gIsLoadingRuleList = NO;
    gEclipseContentRuleList = list;
    for (WKWebView *wv in [gAdblockWantedWebViews allObjects]) {
        eclipse_attach_rule_list(wv);
    }
}

static void eclipse_remove_stale_rule_lists(WKContentRuleListStore *store, NSString *current) {
    [store getAvailableContentRuleListIdentifiers:^(NSArray<NSString *> *identifiers) {
        for (NSString *identifier in identifiers) {
            if ([identifier hasPrefix:@"eclipse-native-shields"] && ![identifier isEqualToString:current]) {
                [store removeContentRuleListForIdentifier:identifier completionHandler:^(NSError *error) {
                    (void)error;
                }];
            }
        }
    }];
}

static void eclipse_compile_content_rule_list_if_needed(void) {
    if (gEclipseContentRuleList != nil || gIsLoadingRuleList || gRuleListFailed) return;
    gIsLoadingRuleList = YES;
    if (gAdblockWantedWebViews == nil) {
        gAdblockWantedWebViews = [NSHashTable weakObjectsHashTable];
        gAdblockAttachedWebViews = [NSHashTable weakObjectsHashTable];
    }

    WKContentRuleListStore *store = [WKContentRuleListStore defaultStore];
    NSString *json = eclipse_native_shields_json();
    NSString *identifier = eclipse_rule_list_identifier(json);

    // A list compiled by an earlier launch is reused without recompiling.
    [store lookUpContentRuleListForIdentifier:identifier
                            completionHandler:^(WKContentRuleList *cached, NSError *lookupError) {
        (void)lookupError;
        if (cached != nil) {
            eclipse_rule_list_ready(cached);
            return;
        }
        [store compileContentRuleListForIdentifier:identifier
                            encodedContentRuleList:json
                                 completionHandler:^(WKContentRuleList *list, NSError *error) {
            if (error != nil || list == nil) {
                // Never retried: the rules are static, so a failure would
                // repeat on every attempt.
                gIsLoadingRuleList = NO;
                gRuleListFailed = YES;
                fprintf(stderr, "[eclipse] native ad blocker disabled: rule list failed to compile: %s\n",
                        error != nil ? error.localizedDescription.UTF8String : "unknown error");
                return;
            }
            eclipse_rule_list_ready(list);
            eclipse_remove_stale_rule_lists(store, identifier);
        }];
    }];
}

/* Attaches or detaches the rule list for one webview, only when its state
   actually changes, so page loads and settings pushes cause no churn. */
void eclipse_set_native_adblocker_enabled(void *viewPtr, int enabled) {
    if (viewPtr == NULL) return;
    @autoreleasepool {
        if (gAdblockWantedWebViews == nil) {
            gAdblockWantedWebViews = [NSHashTable weakObjectsHashTable];
            gAdblockAttachedWebViews = [NSHashTable weakObjectsHashTable];
        }

        NSView *view = (__bridge NSView *)viewPtr;
        WKWebView *wv = eclipse_find_wkwebview(view);
        if (wv == nil) return;

        if (enabled) {
            [gAdblockWantedWebViews addObject:wv];
            if (gEclipseContentRuleList != nil) {
                eclipse_attach_rule_list(wv);
            } else {
                eclipse_compile_content_rule_list_if_needed();
            }
        } else {
            [gAdblockWantedWebViews removeObject:wv];
            eclipse_detach_rule_list(wv);
        }
    }
}

void eclipse_install_native_adblocker(void *viewPtr, int enabled) {
    eclipse_set_native_adblocker_enabled(viewPtr, enabled);
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

static NSString *const kEclipsePositionAnimKey = @"eclipsePositionAnim";
static NSString *const kEclipseSizeAnimKey = @"eclipseBoundsAnim";

static void eclipse_cancel_frame_animations(CALayer *layer) {
    [layer removeAnimationForKey:kEclipsePositionAnimKey];
    [layer removeAnimationForKey:kEclipseSizeAnimKey];
    // Implicit animations an NSView animator may have added.
    [layer removeAnimationForKey:@"position"];
    [layer removeAnimationForKey:@"bounds"];
}

/* The model frame is always set immediately; the motion is drawn by additive
   animations that decay from the old on-screen offset to zero. A later frame
   change (another call here, or a plain setFrame from wry's set_bounds) then
   lands on its own target instead of the view finishing a stale animation
   and snapping. */
void eclipse_animate_view_frame(void *viewPtr, double x, double y, double width, double height, double duration) {
    if (viewPtr == NULL) return;
    @autoreleasepool {
        NSView *view = (__bridge NSView *)viewPtr;
        NSView *superview = [view superview];
        if (superview == nil) return;
        // WebKit moves a webview into its own window for element (video)
        // fullscreen; browser layout must not resize it there.
        NSWindow *browser = gBrowserWindow;
        if (browser != nil && view.window != nil && view.window != browser) return;
        view.wantsLayer = YES;

        CGFloat actualY = y;
        if (![superview isFlipped]) {
            actualY = superview.frame.size.height - y - height;
        }
        NSRect targetFrame = NSMakeRect(x, actualY, width, height);
        CALayer *layer = view.layer;

        if (duration <= 0.001 || layer == nil) {
            [CATransaction begin];
            [CATransaction setDisableActions:YES];
            if (layer != nil) eclipse_cancel_frame_animations(layer);
            [view setFrame:targetFrame];
            [CATransaction commit];
            return;
        }

        CALayer *pres = [layer presentationLayer] ?: layer;
        CGPoint oldPos = pres.position;
        CGSize oldSize = pres.bounds.size;

        [CATransaction begin];
        [CATransaction setDisableActions:YES];
        eclipse_cancel_frame_animations(layer);
        [view setFrame:targetFrame];
        [CATransaction commit];

        CGPoint newPos = layer.position;
        CGSize newSize = layer.bounds.size;
        CAMediaTimingFunction *springCurve =
            [CAMediaTimingFunction functionWithControlPoints:(float)0.22 :(float)1.0 :(float)0.36 :(float)1.0];

        if (fabs(oldPos.x - newPos.x) > 0.5 || fabs(oldPos.y - newPos.y) > 0.5) {
            CABasicAnimation *posAnim = [CABasicAnimation animationWithKeyPath:@"position"];
            posAnim.fromValue = [NSValue valueWithPoint:NSMakePoint(oldPos.x - newPos.x, oldPos.y - newPos.y)];
            posAnim.toValue = [NSValue valueWithPoint:NSZeroPoint];
            posAnim.additive = YES;
            posAnim.duration = duration;
            posAnim.timingFunction = springCurve;
            [layer addAnimation:posAnim forKey:kEclipsePositionAnimKey];
        }
        if (fabs(oldSize.width - newSize.width) > 0.5 || fabs(oldSize.height - newSize.height) > 0.5) {
            CABasicAnimation *sizeAnim = [CABasicAnimation animationWithKeyPath:@"bounds.size"];
            sizeAnim.fromValue = [NSValue valueWithSize:NSMakeSize(oldSize.width - newSize.width,
                                                                   oldSize.height - newSize.height)];
            sizeAnim.toValue = [NSValue valueWithSize:NSZeroSize];
            sizeAnim.additive = YES;
            sizeAnim.duration = duration;
            sizeAnim.timingFunction = springCurve;
            [layer addAnimation:sizeAnim forKey:kEclipseSizeAnimKey];
        }
    }
}

/* ------------------------------------------------------------- Edge hover */
/* Expansion is detected from mouse-moved events of the browser window, which
   arrive only while the app is active. The poll timer runs only while the
   sidebar is hover-expanded (to notice the pointer leaving the window) and
   the browser window is key, so an idle or background app never wakes up. */
static double gSidebarWidth = 232.0;
static BOOL gIsHoverExpanded = NO;     // Mirrors Rust's state (eclipse_set_hover_expanded).
static BOOL gExpandRequested = NO;     // Expand fired for this edge visit; awaiting Rust.
static BOOL gCollapseRequested = NO;   // Collapse fired; Rust may ignore it while held open.
static CFAbsoluteTime gOutsideSince = 0.0;
static void (*gEdgeExpandCallback)(void) = NULL;
static void (*gEdgeCollapseCallback)(void) = NULL;
static id gLocalMouseMonitor = nil;
static NSTimer *gMousePollTimer = nil;

static BOOL eclipse_edge_tracking_allowed(NSWindow *window) {
    return window != nil && [NSApp isActive] && [window isKeyWindow];
}

/* A mouse button held down (a drag, or a click that is still in progress) or
   a menu being tracked (a context menu or <select> popup) keeps the sidebar
   open; the timer collapses it once that ends. */
static BOOL eclipse_pointer_interaction_in_progress(void) {
    if ([NSEvent pressedMouseButtons] != 0) return YES;
    NSRunLoopMode mode = [[NSRunLoop currentRunLoop] currentMode];
    return mode != nil && [mode isEqualToString:NSEventTrackingRunLoopMode];
}

static void eclipse_fire_collapse(void) {
    gCollapseRequested = YES;
    gOutsideSince = 0.0;
    gEdgeCollapseCallback();
}

static void evaluateEdgeHoverAtWindowPoint(NSPoint pt, CGFloat winH, BOOL immediate) {
    if (gEdgeExpandCallback == NULL || gEdgeCollapseCallback == NULL) return;

    if (!gIsHoverExpanded) {
        gOutsideSince = 0.0;
        gCollapseRequested = NO;
        BOOL atEdge = (pt.x >= -2.0 && pt.x <= 12.0 && pt.y >= 0.0 && pt.y <= winH);
        if (!atEdge) {
            gExpandRequested = NO;
        } else if (!gExpandRequested) {
            // Fired once per visit to the edge: Rust ignores it while the
            // sidebar is docked, and must not be flooded with requests.
            gExpandRequested = YES;
            gEdgeExpandCallback();
        }
        return;
    }

    gExpandRequested = NO;
    BOOL isOutside = (pt.x > (gSidebarWidth + 24.0) || pt.x < -20.0 || pt.y < -10.0 || pt.y > (winH + 10.0));
    if (!isOutside) {
        gOutsideSince = 0.0;
        gCollapseRequested = NO;
        return;
    }
    if (gCollapseRequested) return;
    if (eclipse_pointer_interaction_in_progress()) {
        gOutsideSince = 0.0;
        return;
    }
    if (immediate) {
        eclipse_fire_collapse();
        return;
    }
    CFAbsoluteTime now = CFAbsoluteTimeGetCurrent();
    if (gOutsideSince <= 0.0) {
        gOutsideSince = now;
    } else if ((now - gOutsideSince) >= 0.075) {
        eclipse_fire_collapse();
    }
}

static void checkMousePosition(BOOL immediate) {
    NSWindow *window = eclipse_browser_window();
    if (!eclipse_edge_tracking_allowed(window)) return;
    NSPoint screenPt = [NSEvent mouseLocation];
    NSRect windowFrame = [window frame];
    NSPoint pt = NSMakePoint(screenPt.x - windowFrame.origin.x, screenPt.y - windowFrame.origin.y);
    evaluateEdgeHoverAtWindowPoint(pt, windowFrame.size.height, immediate);
}

static void eclipse_update_poll_timer(void) {
    BOOL wanted = gEdgeCollapseCallback != NULL && gIsHoverExpanded &&
                  eclipse_edge_tracking_allowed(eclipse_browser_window());
    if (wanted && gMousePollTimer == nil) {
        gMousePollTimer = [NSTimer timerWithTimeInterval:0.032
                                                 repeats:YES
                                                   block:^(NSTimer * _Nonnull timer) {
            (void)timer;
            checkMousePosition(NO);
            eclipse_update_poll_timer();
        }];
        gMousePollTimer.tolerance = 0.008;
        // Common modes keep it firing while a menu or drag is being tracked.
        [[NSRunLoop mainRunLoop] addTimer:gMousePollTimer forMode:NSRunLoopCommonModes];
    } else if (!wanted && gMousePollTimer != nil) {
        [gMousePollTimer invalidate];
        gMousePollTimer = nil;
        gOutsideSince = 0.0;
    }
}

void eclipse_set_hover_expanded(int is_expanded, double sidebar_width) {
    gIsHoverExpanded = is_expanded ? YES : NO;
    gExpandRequested = NO;
    gCollapseRequested = NO;
    gOutsideSince = 0.0;
    if (sidebar_width > 50.0) {
        gSidebarWidth = sidebar_width;
    }
    eclipse_update_poll_timer();
}

/* Re-evaluates the pointer now (for example when the sidebar no longer needs
   to be held open) and collapses at once if it is outside the sidebar. */
void eclipse_recheck_hover(void) {
    gCollapseRequested = NO;
    gOutsideSince = 0.0;
    if (gIsHoverExpanded) {
        checkMousePosition(YES);
    }
    eclipse_update_poll_timer();
}

void eclipse_install_edge_monitor(void (*expand_cb)(void), void (*collapse_cb)(void)) {
    gEdgeExpandCallback = expand_cb;
    gEdgeCollapseCallback = collapse_cb;

    if (gLocalMouseMonitor == nil) {
        gLocalMouseMonitor = [NSEvent addLocalMonitorForEventsMatchingMask:NSEventMaskMouseMoved | NSEventMaskLeftMouseDragged
                                                                   handler:^NSEvent *(NSEvent *event) {
            NSWindow *window = [event window];
            // Native fullscreen included: the sidebar is reachable there too.
            if (window != nil && window == eclipse_browser_window() && eclipse_edge_tracking_allowed(window)) {
                evaluateEdgeHoverAtWindowPoint([event locationInWindow], window.frame.size.height, NO);
            }
            return event;
        }];

        NSNotificationCenter *center = [NSNotificationCenter defaultCenter];
        void (^refresh)(NSNotification *) = ^(NSNotification *note) {
            (void)note;
            gOutsideSince = 0.0;
            eclipse_update_poll_timer();
        };
        for (NSNotificationName name in @[ NSApplicationDidBecomeActiveNotification,
                                           NSApplicationDidResignActiveNotification,
                                           NSWindowDidBecomeKeyNotification,
                                           NSWindowDidResignKeyNotification ]) {
            [center addObserverForName:name object:nil queue:[NSOperationQueue mainQueue] usingBlock:refresh];
        }
    }
    eclipse_update_poll_timer();
}

/* ------------------------------------------------------------- Navigation failures */
/* wry's navigation delegate has no didFail* methods, so a failed load would
   leave the tab spinning on a blank page. The methods are added to the class
   of each tab's existing delegate (never replacing one it implements) and
   report every error to Rust with the tab id registered for that webview.
   Rust decides what an error means (tab::classify_navigation_error): a
   superseded load is ignored, a download or a refused scheme only ends the
   pending load, a media document that took over the load counts as finished,
   and only real failures show the error page. */

typedef void (*EclipseNavigationCallback)(uint64_t tabId, int kind, const char *domain, int64_t code,
                                          const char *description, const char *url);

static EclipseNavigationCallback gNavigationCallback = NULL;
static NSMapTable<WKWebView *, NSNumber *> *gTabIdsByWebView = nil;

static void eclipse_report_navigation_failure(WKWebView *webView, int kind, NSError *error) {
    if (gNavigationCallback == NULL || webView == nil || error == nil) return;
    NSNumber *tabId = [gTabIdsByWebView objectForKey:webView];
    if (tabId == nil) return;

    NSURL *failingURL = error.userInfo[NSURLErrorFailingURLErrorKey];
    NSString *url = [failingURL isKindOfClass:[NSURL class]] ? failingURL.absoluteString : nil;
    if (url.length == 0) {
        NSString *failingString = error.userInfo[NSURLErrorFailingURLStringErrorKey];
        url = [failingString isKindOfClass:[NSString class]] ? failingString : nil;
    }
    if (url.length == 0) {
        url = webView.URL.absoluteString;
    }

    NSString *domain = error.domain ?: @"";
    NSString *description = error.localizedDescription ?: @"";
    gNavigationCallback(tabId.unsignedLongLongValue, kind, domain.UTF8String, (int64_t)error.code,
                        description.UTF8String, (url ?: @"").UTF8String);
}

static void eclipse_did_fail_provisional_navigation(id self, SEL _cmd, WKWebView *webView,
                                                    WKNavigation *navigation, NSError *error) {
    (void)self; (void)_cmd; (void)navigation;
    eclipse_report_navigation_failure(webView, 0, error);
}

static void eclipse_did_fail_navigation(id self, SEL _cmd, WKWebView *webView,
                                        WKNavigation *navigation, NSError *error) {
    (void)self; (void)_cmd; (void)navigation;
    eclipse_report_navigation_failure(webView, 1, error);
}

/* ------------------------------------------------------------- Navigation policy context */
/* wry's navigation handler only receives the URL. The delegate's
   decidePolicyForNavigationAction: is wrapped so that, while the original
   implementation (and with it the Rust handler) runs, Rust can ask whether
   the decision is for a tab's main frame and whether it is a back/forward
   history navigation. Everything runs on the main thread. */

enum {
    ECLIPSE_POLICY_MAIN_FRAME = 1,
    ECLIPSE_POLICY_BACK_FORWARD = 2,
    /* A script in another webview started the navigation (a Little Eclipse
       popup setting window.opener.location, or targeting the tab by name). */
    ECLIPSE_POLICY_OTHER_VIEW = 4,
};

typedef void (*EclipseDecidePolicyIMP)(id, SEL, WKWebView *, WKNavigationAction *,
                                       void (^)(WKNavigationActionPolicy));

static int gPolicyContext = 0;
static NSMutableDictionary<NSString *, NSValue *> *gOriginalDecidePolicy = nil;

static IMP eclipse_original_decide_policy(Class cls) {
    for (Class current = cls; current != Nil; current = class_getSuperclass(current)) {
        NSValue *value = gOriginalDecidePolicy[NSStringFromClass(current)];
        if (value != nil) return (IMP)value.pointerValue;
    }
    return NULL;
}

static void eclipse_decide_policy(id self, SEL _cmd, WKWebView *webView, WKNavigationAction *action,
                                  void (^decisionHandler)(WKNavigationActionPolicy)) {
    IMP original = eclipse_original_decide_policy(object_getClass(self));
    if (original == NULL) {
        // Cannot happen (the wrapper is only installed with its original);
        // fail closed rather than skip the navigation policy.
        decisionHandler(WKNavigationActionPolicyCancel);
        return;
    }
    int context = 0;
    WKFrameInfo *target = action.targetFrame;
    if (target != nil && target.isMainFrame) {
        context |= ECLIPSE_POLICY_MAIN_FRAME;
        if (action.navigationType == WKNavigationTypeBackForward) {
            context |= ECLIPSE_POLICY_BACK_FORWARD;
        }
    }
    // The source frame is nil or this webview's own for the browser's loads
    // and the tab's own pages. A frame whose webview is gone counts as
    // another view.
    WKFrameInfo *source = action.sourceFrame;
    if (source != nil && source.webView != webView) {
        context |= ECLIPSE_POLICY_OTHER_VIEW;
    }
    int previous = gPolicyContext;
    gPolicyContext = context;
    ((EclipseDecidePolicyIMP)original)(self, _cmd, webView, action, decisionHandler);
    gPolicyContext = previous;
}

static void eclipse_wrap_decide_policy(Class cls) {
    SEL sel = @selector(webView:decidePolicyForNavigationAction:decisionHandler:);
    Method method = class_getInstanceMethod(cls, sel);
    if (method == NULL) return;
    IMP current = method_getImplementation(method);
    if (current == (IMP)eclipse_decide_policy) return;
    if (gOriginalDecidePolicy == nil) {
        gOriginalDecidePolicy = [NSMutableDictionary dictionary];
    }
    gOriginalDecidePolicy[NSStringFromClass(cls)] = [NSValue valueWithPointer:(const void *)current];
    class_replaceMethod(cls, sel, (IMP)eclipse_decide_policy, method_getTypeEncoding(method));
}

int eclipse_navigation_policy_context(void) {
    return gPolicyContext;
}

/* ------------------------------------------------------------- Popup guards */
/* Little Eclipse windows (see below) rely on three things wry does not
   provide. Each is added by wrapping a method of one of wry's classes, like
   the policy context above:

   - Whether a window.open came with a user gesture. wry's new-window handler
     only receives the URL and the window features, so the UI delegate's
     createWebView… method is wrapped to publish the navigation action's
     gesture state while it (and with it the Rust handler) runs. Without a
     gesture a popup opens as a tab, never as a window of its own.
   - wry's eclipse:// WKURLSchemeHandler takes the requesting view to be a
     WryWebView and reaches into that class's instance variables. A popup's
     plain WKWebView inherits the handler through its configuration, so the
     handler's start and stop methods are wrapped to fail every task from any
     other view before wry sees it (popups never load eclipse:// resources).
   - A popup shares its opener tab's WKUserContentController, so its script
     messages reach the tab's IPC handler. wry's message handler is wrapped
     so that, while it runs, Rust can ask whether the message came from a
     Little Eclipse webview and never take it as the tab's own page.

   Little Eclipse windows only open once all three are in place. Everything
   runs on the main thread. */

static BOOL eclipse_perf_enabled(void) {
    static int enabled = -1;
    if (enabled < 0) enabled = getenv("ECLIPSE_PERF") != NULL ? 1 : 0;
    return enabled == 1;
}

static NSMutableDictionary<NSString *, NSValue *> *gOriginalMethods = nil;

static NSString *eclipse_original_method_key(Class cls, SEL sel) {
    return [NSString stringWithFormat:@"%@ %@", NSStringFromClass(cls), NSStringFromSelector(sel)];
}

/* The implementation `sel` had before eclipse_wrap_method replaced it on
   `cls` or one of its superclasses. */
static IMP eclipse_original_method(Class cls, SEL sel) {
    for (Class current = cls; current != Nil; current = class_getSuperclass(current)) {
        NSValue *value = gOriginalMethods[eclipse_original_method_key(current, sel)];
        if (value != nil) return (IMP)value.pointerValue;
    }
    return NULL;
}

/* Puts `wrapper` in place of `cls`'s implementation of `sel` and keeps the
   original for eclipse_original_method. YES when the wrapper is in place
   (also when it already was). */
static BOOL eclipse_wrap_method(Class cls, SEL sel, IMP wrapper) {
    if (cls == Nil) return NO;
    Method method = class_getInstanceMethod(cls, sel);
    if (method == NULL) return NO;
    IMP current = method_getImplementation(method);
    if (current == wrapper) return YES;
    if (gOriginalMethods == nil) gOriginalMethods = [NSMutableDictionary dictionary];
    gOriginalMethods[eclipse_original_method_key(cls, sel)] = [NSValue valueWithPointer:(const void *)current];
    class_replaceMethod(cls, sel, wrapper, method_getTypeEncoding(method));
    return YES;
}

static BOOL eclipse_method_is_wrapped(Class cls, SEL sel, IMP wrapper) {
    return cls != Nil && class_getMethodImplementation(cls, sel) == wrapper;
}

/* -- user gesture */

enum {
    ECLIPSE_POPUP_REQUEST = 1,
    ECLIPSE_POPUP_USER_INITIATED = 2,
    ECLIPSE_POPUP_WANTS_POPUP = 4,
};

static int gPopupRequestContext = 0;
/* The window features of that request (a width without a height, say). */
static WKWindowFeatures *gPopupRequestFeatures = nil;

/* WebKit's -[WKNavigationAction _isUserInitiated] (SPI, present since macOS
   10.12): the navigation, or the window.open, runs from a user gesture. An
   unknown answer is "no", so a popup then opens as a tab. */
static BOOL eclipse_action_user_initiated(WKNavigationAction *action) {
    SEL sel = NSSelectorFromString(@"_isUserInitiated");
    if (action == nil || ![action respondsToSelector:sel]) return NO;
    return ((BOOL (*)(id, SEL))objc_msgSend)(action, sel);
}

/* Whether window.open asked for a popup window rather than a tab, by the
   HTML rule browsers use (WebKit's -[WKWindowFeatures _wantsPopup]): a
   "popup" feature, a size or position, or turned-off toolbars/location/menu
   bar. window.open(url) and window.open(url, name) with no features, and
   target=_blank links, are tabs. */
static BOOL eclipse_features_want_popup(WKWindowFeatures *features) {
    if (features == nil) return NO;
    SEL sel = NSSelectorFromString(@"_wantsPopup");
    if ([features respondsToSelector:sel]) return ((BOOL (*)(id, SEL))objc_msgSend)(features, sel);
    // Older WebKit: any size or position, or toolbars explicitly off.
    if (features.width != nil || features.height != nil || features.x != nil || features.y != nil) return YES;
    return features.toolbarsVisibility != nil && !features.toolbarsVisibility.boolValue;
}

typedef WKWebView *(*EclipseCreateWebViewIMP)(id, SEL, WKWebView *, WKWebViewConfiguration *,
                                              WKNavigationAction *, WKWindowFeatures *);

static WKWebView *eclipse_create_web_view(id self, SEL _cmd, WKWebView *webView,
                                          WKWebViewConfiguration *configuration,
                                          WKNavigationAction *action, WKWindowFeatures *features) {
    IMP original = eclipse_original_method(object_getClass(self), _cmd);
    if (original == NULL) return nil;
    int previous = gPopupRequestContext;
    WKWindowFeatures *previousFeatures = gPopupRequestFeatures;
    gPopupRequestFeatures = features;
    gPopupRequestContext = ECLIPSE_POPUP_REQUEST |
                           (eclipse_action_user_initiated(action) ? ECLIPSE_POPUP_USER_INITIATED : 0) |
                           (eclipse_features_want_popup(features) ? ECLIPSE_POPUP_WANTS_POPUP : 0);
    WKWebView *created = nil;
    @try {
        created = ((EclipseCreateWebViewIMP)original)(self, _cmd, webView, configuration, action, features);
    } @finally {
        // Never left set for a later request.
        gPopupRequestContext = previous;
        gPopupRequestFeatures = previousFeatures;
    }
    return created;
}

static SEL eclipse_create_web_view_selector(void) {
    return @selector(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:);
}

/* -- eclipse:// requests from views that are not wry's */

/* The class of the tabs' webviews (wry's WryWebView), the only views wry's
   scheme handler may serve. */
static Class gWryWebViewClass = Nil;

typedef void (*EclipseSchemeTaskIMP)(id, SEL, WKWebView *, id<WKURLSchemeTask>);

static BOOL eclipse_is_wry_webview(WKWebView *webView) {
    Class cls = gWryWebViewClass;
    return cls != Nil && webView != nil && [webView isKindOfClass:cls];
}

static void eclipse_start_scheme_task(id self, SEL _cmd, WKWebView *webView, id<WKURLSchemeTask> task) {
    IMP original = eclipse_original_method(object_getClass(self), _cmd);
    if (original != NULL && eclipse_is_wry_webview(webView)) {
        ((EclipseSchemeTaskIMP)original)(self, _cmd, webView, task);
        return;
    }
    if (eclipse_perf_enabled()) {
        fprintf(stderr, "[little] refused %s request from a popup\n",
                task.request.URL.scheme.UTF8String ?: "custom-scheme");
    }
    @try {
        [task didFailWithError:[NSError errorWithDomain:NSURLErrorDomain
                                                   code:NSURLErrorUnsupportedURL
                                               userInfo:nil]];
    } @catch (NSException *exception) {
        (void)exception;  // WebKit already stopped the task.
    }
}

static void eclipse_stop_scheme_task(id self, SEL _cmd, WKWebView *webView, id<WKURLSchemeTask> task) {
    IMP original = eclipse_original_method(object_getClass(self), _cmd);
    // A task from another view was failed when it started: wry never saw it.
    if (original != NULL && eclipse_is_wry_webview(webView)) {
        ((EclipseSchemeTaskIMP)original)(self, _cmd, webView, task);
    }
}

/* Whether wry's handler for `scheme` in `configuration` (if any) is guarded. */
static BOOL eclipse_scheme_handler_guarded(WKWebViewConfiguration *configuration, NSString *scheme) {
    id<WKURLSchemeHandler> handler = [configuration urlSchemeHandlerForURLScheme:scheme];
    if (handler == nil) return YES;
    Class cls = object_getClass(handler);
    return eclipse_method_is_wrapped(cls, @selector(webView:startURLSchemeTask:), (IMP)eclipse_start_scheme_task) &&
           eclipse_method_is_wrapped(cls, @selector(webView:stopURLSchemeTask:), (IMP)eclipse_stop_scheme_task);
}

/* -- script messages from Little Eclipse webviews */

/* Every Little Eclipse webview, until it is deallocated. */
static NSHashTable<WKWebView *> *gLittleWebViews = nil;
static int gScriptMessageFromLittleWindow = 0;

typedef void (*EclipseScriptMessageIMP)(id, SEL, WKUserContentController *, WKScriptMessage *);

static void eclipse_did_receive_script_message(id self, SEL _cmd, WKUserContentController *controller,
                                               WKScriptMessage *message) {
    IMP original = eclipse_original_method(object_getClass(self), _cmd);
    if (original == NULL) return;
    // A message from a view that is already gone is not trusted as the tab's
    // page either once popups exist.
    WKWebView *sender = message.frameInfo.webView;
    BOOL fromLittle = gLittleWebViews != nil && (sender == nil || [gLittleWebViews containsObject:sender]);
    int previous = gScriptMessageFromLittleWindow;
    gScriptMessageFromLittleWindow = fromLittle ? 1 : 0;
    @try {
        ((EclipseScriptMessageIMP)original)(self, _cmd, controller, message);
    } @finally {
        gScriptMessageFromLittleWindow = previous;
    }
}

/* 1 while wry's IPC handler runs for a message a Little Eclipse webview
   posted, else 0. */
int eclipse_script_message_from_little_window(void) {
    return gScriptMessageFromLittleWindow;
}

/* wry's WKScriptMessageHandler class, WryWebViewDelegate. wry's classes are
   made at run time by objc2, named "<module path>::<type><crate version>"
   (objc2's documented default), so its name follows from the name of the
   tabs' webview class, wry's WryWebView, defined in a sibling module. Nil
   when the names do not fit that pattern or the class is not a message
   handler (popups then open as tabs). */
static Class eclipse_find_wry_ipc_class(Class wryWebViewClass) {
    NSString *viewName = NSStringFromClass(wryWebViewClass);
    NSString *viewPart = @"::wry_web_view::WryWebView";
    NSRange range = [viewName rangeOfString:viewPart options:NSBackwardsSearch];
    if (range.location == NSNotFound) return Nil;
    NSString *prefix = [viewName substringToIndex:range.location];
    NSString *version = [viewName substringFromIndex:NSMaxRange(range)];
    NSString *name = [NSString stringWithFormat:@"%@::wry_web_view_delegate::WryWebViewDelegate%@", prefix, version];
    Class cls = NSClassFromString(name);
    if (cls == Nil || !class_conformsToProtocol(cls, @protocol(WKScriptMessageHandler)) ||
        !class_respondsToSelector(cls, @selector(userContentController:didReceiveScriptMessage:))) {
        return Nil;
    }
    return cls;
}

/* -- installation */

static BOOL gLittleGuardsReady = NO;

/* Installs the guards from a tab's webview (a WryWebView whose UI delegate
   was already hooked). Little Eclipse windows stay off until this succeeds. */
static void eclipse_little_guards_unavailable(const char *why) {
    static BOOL logged = NO;
    if (logged || !eclipse_perf_enabled()) return;
    logged = YES;
    fprintf(stderr, "[little] guards unavailable (%s): popups open as tabs\n", why);
}

static void eclipse_little_install_guards(WKWebView *tabView) {
    if (gLittleGuardsReady || tabView == nil) return;
    Class viewClass = [tabView class];
    if (viewClass == [WKWebView class] || ![viewClass isSubclassOfClass:[WKWebView class]]) {
        eclipse_little_guards_unavailable("tab view class");
        return;
    }
    if (gWryWebViewClass == Nil) {
        // Set before the scheme handler is wrapped: the wrapper serves only
        // views of this class.
        gWryWebViewClass = viewClass;
    } else if (gWryWebViewClass != viewClass) {
        eclipse_little_guards_unavailable("tab view class changed");
        return;
    }
    id uiDelegate = tabView.UIDelegate;
    if (uiDelegate == nil ||
        !eclipse_method_is_wrapped(object_getClass(uiDelegate), eclipse_create_web_view_selector(),
                                   (IMP)eclipse_create_web_view)) {
        eclipse_little_guards_unavailable("UI delegate");
        return;
    }
    id<WKURLSchemeHandler> handler = [tabView.configuration urlSchemeHandlerForURLScheme:@"eclipse"];
    if (handler != nil) {
        Class handlerClass = object_getClass(handler);
        if (!eclipse_wrap_method(handlerClass, @selector(webView:startURLSchemeTask:),
                                 (IMP)eclipse_start_scheme_task) ||
            !eclipse_wrap_method(handlerClass, @selector(webView:stopURLSchemeTask:),
                                 (IMP)eclipse_stop_scheme_task)) {
            eclipse_little_guards_unavailable("scheme handler");
            return;
        }
    }
    // wry's WKScriptMessageHandler (the IPC handler of every webview).
    Class ipcClass = eclipse_find_wry_ipc_class(viewClass);
    if (ipcClass == Nil ||
        !eclipse_wrap_method(ipcClass, @selector(userContentController:didReceiveScriptMessage:),
                             (IMP)eclipse_did_receive_script_message)) {
        eclipse_little_guards_unavailable("IPC handler");
        return;
    }
    gLittleGuardsReady = YES;
    if (eclipse_perf_enabled()) {
        fprintf(stderr, "[little] guards installed\n");
    }
}

/* ------------------------------------------------------------- JavaScript dialogs & page print */
/* wry's UI delegate lacks alert/confirm/prompt (WebKit then silently returns
   cancel) and the print callback, so they are added the same way. */

static NSString *eclipse_dialog_title(WKFrameInfo *frame) {
    NSURL *url = frame.request.URL;
    if ([url.scheme isEqualToString:@"eclipse"]) return @"Eclipse";
    NSString *host = frame.securityOrigin.host;
    if (host.length == 0) host = url.host;
    if (host.length == 0) return @"This page says:";
    return [NSString stringWithFormat:@"The page at %@ says:", host];
}

static NSAlert *eclipse_dialog_alert(WKFrameInfo *frame, NSString *message) {
    NSAlert *alert = [[NSAlert alloc] init];
    alert.messageText = eclipse_dialog_title(frame);
    NSString *text = message ?: @"";
    if (text.length > 2000) {
        text = [[text substringToIndex:2000] stringByAppendingString:@"…"];
    }
    alert.informativeText = text;
    return alert;
}

/* Shows the alert as a sheet on the webview's window; `done` always runs
   exactly once, also when the sheet is dismissed because its window closes.
   A hidden webview (a background or closing tab) gets an immediate "Cancel":
   its sheet would block the whole window over another tab. */
static void eclipse_present_dialog(WKWebView *webView, NSAlert *alert, void (^done)(NSModalResponse)) {
    if (webView.isHiddenOrHasHiddenAncestor) {
        done(NSModalResponseCancel);
        return;
    }
    NSWindow *window = webView.window;
    if (window != nil) {
        [alert beginSheetModalForWindow:window completionHandler:done];
    } else {
        done([alert runModal]);
    }
}

static void eclipse_run_alert_panel(id self, SEL _cmd, WKWebView *webView, NSString *message,
                                    WKFrameInfo *frame, void (^completionHandler)(void)) {
    (void)self; (void)_cmd;
    NSAlert *alert = eclipse_dialog_alert(frame, message);
    [alert addButtonWithTitle:@"OK"];
    eclipse_present_dialog(webView, alert, ^(NSModalResponse response) {
        (void)response;
        completionHandler();
    });
}

static void eclipse_run_confirm_panel(id self, SEL _cmd, WKWebView *webView, NSString *message,
                                      WKFrameInfo *frame, void (^completionHandler)(BOOL)) {
    (void)self; (void)_cmd;
    NSAlert *alert = eclipse_dialog_alert(frame, message);
    [alert addButtonWithTitle:@"OK"];
    [alert addButtonWithTitle:@"Cancel"];
    eclipse_present_dialog(webView, alert, ^(NSModalResponse response) {
        completionHandler(response == NSAlertFirstButtonReturn);
    });
}

static void eclipse_run_text_input_panel(id self, SEL _cmd, WKWebView *webView, NSString *prompt,
                                         NSString *defaultText, WKFrameInfo *frame,
                                         void (^completionHandler)(NSString *)) {
    (void)self; (void)_cmd;
    NSAlert *alert = eclipse_dialog_alert(frame, prompt);
    [alert addButtonWithTitle:@"OK"];
    [alert addButtonWithTitle:@"Cancel"];
    NSTextField *field = [[NSTextField alloc] initWithFrame:NSMakeRect(0, 0, 300, 24)];
    field.stringValue = defaultText ?: @"";
    alert.accessoryView = field;
    alert.window.initialFirstResponder = field;
    eclipse_present_dialog(webView, alert, ^(NSModalResponse response) {
        completionHandler(response == NSAlertFirstButtonReturn ? field.stringValue : nil);
    });
}

/* Web views with a print queued or showing. window.print() is not
   throttled, and each queued sheet would attach to the window as soon as the
   one before it ends, so a page could stack sheets that keep a closed popup
   alive. One print at a time per web view. */
static NSHashTable<WKWebView *> *gPrintingWebViews;

@interface EclipsePrintDelegate : NSObject
@end

@implementation EclipsePrintDelegate
- (void)printOperationDidRun:(NSPrintOperation *)operation success:(BOOL)success contextInfo:(void *)contextInfo {
    (void)operation; (void)success;
    WKWebView *webView = (__bridge_transfer WKWebView *)contextInfo;
    [gPrintingWebViews removeObject:webView];
}
@end

/* Shows the print sheet for `webView` on the next turn of the run loop. */
static void eclipse_print_webview(WKWebView *webView) {
    if (webView == nil) return;
    if (gPrintingWebViews == nil) gPrintingWebViews = [NSHashTable weakObjectsHashTable];
    if ([gPrintingWebViews containsObject:webView]) return;
    [gPrintingWebViews addObject:webView];
    __weak WKWebView *weakWebView = webView;
    dispatch_async(dispatch_get_main_queue(), ^{
        WKWebView *wv = weakWebView;
        if (wv == nil) return;
        NSWindow *window = wv.window;
        // Re-checked now: the window may have closed or gained a dialog.
        if (window == nil || !window.isVisible || window.attachedSheet != nil ||
            ![wv respondsToSelector:@selector(printOperationWithPrintInfo:)]) {
            [gPrintingWebViews removeObject:wv];
            return;
        }
        static EclipsePrintDelegate *delegate;
        if (delegate == nil) delegate = [EclipsePrintDelegate new];
        NSPrintOperation *operation = [wv printOperationWithPrintInfo:[NSPrintInfo sharedPrintInfo]];
        operation.canSpawnSeparateThread = YES;
        [operation runOperationModalForWindow:window
                                     delegate:delegate
                               didRunSelector:@selector(printOperationDidRun:success:contextInfo:)
                                  contextInfo:(__bridge_retained void *)wv];
    });
}

/* Cancels every sheet on `window` (a queued one attaches as soon as the one
   before it ends), with a bound in case a sheet refuses to go. */
static void eclipse_end_all_sheets(NSWindow *window) {
    for (int i = 0; i < 8 && window.attachedSheet != nil; i++) {
        [window endSheet:window.attachedSheet returnCode:NSModalResponseCancel];
    }
}

/* WebKit's private UI-delegate callback for window.print(). WebKit resumes
   the page once this returns, so the print sheet starts on the next turn of
   the run loop, when the page can render for it. */
static void eclipse_print_frame(id self, SEL _cmd, WKWebView *webView, id frame) {
    (void)self; (void)_cmd; (void)frame;
    eclipse_print_webview(webView);
}

/* Adds `imp` for `sel` to `cls` unless the class (or a superclass) already
   responds to it. Returns YES when a method was added. */
static BOOL eclipse_add_method_if_missing(Class cls, SEL sel, IMP imp, const char *types) {
    if (cls == Nil || class_respondsToSelector(cls, sel)) return NO;
    return class_addMethod(cls, sel, imp, types);
}

static void eclipse_hook_delegates(WKWebView *webView) {
    id navigationDelegate = webView.navigationDelegate;
    if (navigationDelegate != nil) {
        Class cls = object_getClass(navigationDelegate);
        BOOL added = NO;
        added |= eclipse_add_method_if_missing(cls, @selector(webView:didFailProvisionalNavigation:withError:),
                                               (IMP)eclipse_did_fail_provisional_navigation, "v@:@@@");
        added |= eclipse_add_method_if_missing(cls, @selector(webView:didFailNavigation:withError:),
                                               (IMP)eclipse_did_fail_navigation, "v@:@@@");
        eclipse_wrap_decide_policy(cls);
        // WebKit records which optional methods a delegate implements when it
        // is assigned, so re-assign it for a class that just gained some.
        if (added) {
            webView.navigationDelegate = navigationDelegate;
        }
    }

    id uiDelegate = webView.UIDelegate;
    if (uiDelegate != nil) {
        Class cls = object_getClass(uiDelegate);
        BOOL added = NO;
        added |= eclipse_add_method_if_missing(
            cls, @selector(webView:runJavaScriptAlertPanelWithMessage:initiatedByFrame:completionHandler:),
            (IMP)eclipse_run_alert_panel, "v@:@@@@?");
        added |= eclipse_add_method_if_missing(
            cls, @selector(webView:runJavaScriptConfirmPanelWithMessage:initiatedByFrame:completionHandler:),
            (IMP)eclipse_run_confirm_panel, "v@:@@@@?");
        added |= eclipse_add_method_if_missing(
            cls, @selector(webView:runJavaScriptTextInputPanelWithPrompt:defaultText:initiatedByFrame:completionHandler:),
            (IMP)eclipse_run_text_input_panel, "v@:@@@@@?");
        added |= eclipse_add_method_if_missing(cls, NSSelectorFromString(@"_webView:printFrame:"),
                                               (IMP)eclipse_print_frame, "v@:@@");
        // Publishes a window.open's user-gesture state to the new-window
        // handler (see Popup guards).
        eclipse_wrap_method(cls, eclipse_create_web_view_selector(), (IMP)eclipse_create_web_view);
        if (added) {
            webView.UIDelegate = uiDelegate;
        }
    }
    // Without a UI delegate WebKit keeps its default behavior (dialogs are
    // dismissed as cancelled); wry always installs one for every webview.
}

void eclipse_install_navigation_hooks(EclipseNavigationCallback callback) {
    gNavigationCallback = callback;
    // Webviews registered before this call were already hooked; their
    // failures are reported from now on.
}

void eclipse_register_tab_view(void *viewPtr, uint64_t tabId) {
    if (viewPtr == NULL) return;
    @autoreleasepool {
        WKWebView *webView = eclipse_find_wkwebview((__bridge NSView *)viewPtr);
        if (webView == nil) return;
        if (gTabIdsByWebView == nil) {
            gTabIdsByWebView = [NSMapTable weakToStrongObjectsMapTable];
        }
        [gTabIdsByWebView setObject:@(tabId) forKey:webView];
        // Checked on every registration: a webview may come with a delegate
        // of a class that has not been hooked yet.
        eclipse_hook_delegates(webView);
        eclipse_little_install_guards(webView);
    }
}

/* ------------------------------------------------------------- Little Eclipse windows */
/* Sign-in and payment popups (window.open with a size, from a web page) open
   in a small native window instead of a tab, so they keep window.opener and
   can post their result back. WebKit requires the popup's WKWebView to be
   built on the configuration it hands the opener's UI delegate; that
   configuration shares the opener tab's WKUserContentController, so the popup
   runs the tab's init scripts and its messages reach the tab's IPC handler
   (Rust only trusts them as an ordinary website).

   A popup opens as a window only for a window.open with a user gesture, and
   at most kEclipseLittleMaxPerTab per tab and kEclipseLittleMaxTotal in all;
   anything else opens as a tab. The guards above keep popups away from wry's
   eclipse:// handler and mark their script messages.

   The window shows the page title, and its subtitle shows the host of the
   document on screen, derived here from WebKit's URL and never from page
   content: the host of a load only once it commits, never of one still
   pending. A popup only ever loads web content: internal eclipse:// pages
   and other schemes are refused by its navigation delegate. Each window
   follows its root opener (the tab's webview): when that webview leaves its
   window (the tab was closed, hibernated or rebuilt) the popup closes. */

@interface EclipseLittleWindow : NSObject <NSWindowDelegate, WKUIDelegate, WKNavigationDelegate>
@property(nonatomic, strong) NSWindow *window;
@property(nonatomic, strong) WKWebView *webView;
/* The tab webview the first popup in this chain was opened from. wry keeps
   a destroyed tab's WKWebView alive, so its detachment, not its
   deallocation, is what signals that the tab is gone. */
@property(nonatomic, weak) WKWebView *opener;
/* The URL of the document on screen, whose host the subtitle shows: nil
   until WebKit confirms one (see -syncDisplayURL). */
@property(nonatomic, copy) NSURL *displayURL;
@property(nonatomic) BOOL observing;
@property(nonatomic) BOOL closed;
@end

static NSMutableSet<EclipseLittleWindow *> *gLittleWindows = nil;
static __weak EclipseLittleWindow *gLastLittleWindow = nil;
static NSTimer *gLittleOpenerTimer = nil;
static void *kEclipseLittleKVOContext = &kEclipseLittleKVOContext;

static const NSUInteger kEclipseLittleMaxPerTab = 3;
static const NSUInteger kEclipseLittleMaxTotal = 10;
static const CGFloat kEclipseLittleMinWidth = 360.0;
static const CGFloat kEclipseLittleMinHeight = 260.0;
/* For a popup that gave no width or height ("popup", or position only). */
static const CGFloat kEclipseLittleDefaultWidth = 500.0;
static const CGFloat kEclipseLittleDefaultHeight = 640.0;
static const NSUInteger kEclipseLittleMaxHostLength = 64;

static BOOL eclipse_is_web_scheme(NSString *scheme) {
    NSString *lower = scheme.lowercaseString;
    return [lower isEqualToString:@"http"] || [lower isEqualToString:@"https"];
}

/* A URL a popup may start with or be created for: http(s), or blank (popups
   often open empty and navigate once the opener has written to them). */
static BOOL eclipse_little_initial_url_allowed(NSURL *url) {
    if (url == nil || url.absoluteString.length == 0) return YES;
    if ([url.absoluteString isEqualToString:@"about:blank"]) return YES;
    return eclipse_is_web_scheme(url.scheme);
}

static BOOL eclipse_same_web_origin(NSURL *a, NSURL *b) {
    if (a == nil || b == nil) return NO;
    if (!eclipse_is_web_scheme(a.scheme) || !eclipse_is_web_scheme(b.scheme)) return NO;
    if ([a.scheme caseInsensitiveCompare:b.scheme] != NSOrderedSame) return NO;
    if (a.host == nil || b.host == nil || [a.host caseInsensitiveCompare:b.host] != NSOrderedSame) return NO;
    NSNumber *pa = a.port, *pb = b.port;
    return (pa == nil && pb == nil) || (pa != nil && pb != nil && [pa isEqualToNumber:pb]);
}

/* What the subtitle shows for the committed document. A long host keeps its
   end (the registrable domain) and loses its start. */
static NSString *eclipse_little_host_label(NSURL *url) {
    if (url == nil) return @"about:blank";
    NSString *scheme = url.scheme.lowercaseString ?: @"";
    if (scheme.length == 0) return @"about:blank";
    if ([scheme isEqualToString:@"about"]) {
        // Never the query or fragment: the page controls those.
        NSString *rest = url.resourceSpecifier ?: @"";
        NSRange cut = [rest rangeOfCharacterFromSet:[NSCharacterSet characterSetWithCharactersInString:@"?#"]];
        if (cut.location != NSNotFound) rest = [rest substringToIndex:cut.location];
        return [@"about:" stringByAppendingString:(rest.length > 0 ? rest : @"blank")];
    }
    if ([scheme isEqualToString:@"blob"]) {
        NSURL *inner = [NSURL URLWithString:url.resourceSpecifier ?: @""];
        if (inner != nil && eclipse_is_web_scheme(inner.scheme)) return eclipse_little_host_label(inner);
        return @"Not Secure — blob:";
    }
    if (!eclipse_is_web_scheme(scheme)) {
        return [NSString stringWithFormat:@"Not Secure — %@:", scheme];
    }
    NSString *host = url.host.lowercaseString ?: @"";
    if (host.length == 0) return [NSString stringWithFormat:@"Not Secure — %@:", scheme];
    if (url.port != nil) host = [NSString stringWithFormat:@"%@:%@", host, url.port];
    if (host.length > kEclipseLittleMaxHostLength) {
        host = [@"…" stringByAppendingString:[host substringFromIndex:host.length - kEclipseLittleMaxHostLength]];
    }
    if ([scheme isEqualToString:@"https"]) return host;
    return [NSString stringWithFormat:@"Not Secure — %@", host];
}

static NSString *eclipse_little_clean_title(NSString *title) {
    if (title.length == 0) return @"";
    NSArray<NSString *> *parts = [title componentsSeparatedByCharactersInSet:[NSCharacterSet whitespaceAndNewlineCharacterSet]];
    NSMutableArray<NSString *> *words = [NSMutableArray arrayWithCapacity:parts.count];
    for (NSString *part in parts) {
        if (part.length > 0) [words addObject:part];
    }
    NSString *clean = [words componentsJoinedByString:@" "];
    if (clean.length > 120) clean = [[clean substringToIndex:120] stringByAppendingString:@"…"];
    return clean;
}

static void eclipse_little_log(NSString *what, EclipseLittleWindow *lw) {
    if (!eclipse_perf_enabled()) return;
    NSString *label = eclipse_little_host_label(lw.displayURL);
    fprintf(stderr, "[little] %s host=%s open=%lu\n", what.UTF8String, label.UTF8String,
            (unsigned long)gLittleWindows.count);
}

static EclipseLittleWindow *eclipse_little_window_create(WKWebViewConfiguration *configuration,
                                                         WKWebView *opener,
                                                         NSSize requested,
                                                         BOOL hasPosition,
                                                         NSPoint topLeft);

static void eclipse_little_check_openers(void);
static NSRect eclipse_little_fit_frame(NSWindow *window, NSRect frame, NSRect visible);

@implementation EclipseLittleWindow

- (void)refreshTitle {
    if (self.closed) return;
    NSString *label = eclipse_little_host_label(self.displayURL);
    NSString *title = eclipse_little_clean_title(self.webView.title);
    if (@available(macOS 11.0, *)) {
        self.window.title = title.length > 0 ? title : label;
        self.window.subtitle = label;
    } else {
        self.window.title = title.length > 0 ? [NSString stringWithFormat:@"%@ — %@", label, title] : label;
    }
}

- (void)observeValueForKeyPath:(NSString *)keyPath
                      ofObject:(id)object
                        change:(NSDictionary<NSKeyValueChangeKey, id> *)change
                       context:(void *)context {
    if (context != kEclipseLittleKVOContext) {
        [super observeValueForKeyPath:keyPath ofObject:object change:change context:context];
        return;
    }
    if (self.closed) return;
    if ([keyPath isEqualToString:@"URL"] || [keyPath isEqualToString:@"loading"]) {
        [self syncDisplayURL];
    }
    [self refreshTitle];
}

/* WebKit's URL already names a pending load while the previous document (or
   the popup's initial blank document) is still on screen, and that load may
   never commit: it can be stopped, fail, or get a 204. So while a load is
   pending only a change within the shown origin is taken (a fragment,
   pushState); a new origin is shown when its document commits
   (didCommitNavigation). Once nothing is loading, WebKit's URL is the
   document on screen, whatever it is: after an abandoned load it is the
   previous document again, and after the opener wrote into the initial
   blank document it is the opener's URL, which that document then has. */
- (void)syncDisplayURL {
    WKWebView *webView = self.webView;
    NSURL *current = webView.URL;
    if (!webView.isLoading) {
        self.displayURL = current;
    } else if (self.displayURL != nil && eclipse_same_web_origin(self.displayURL, current)) {
        self.displayURL = current;
    }
}

- (void)stopObserving {
    if (!self.observing) return;
    self.observing = NO;
    [self.webView removeObserver:self forKeyPath:@"URL" context:kEclipseLittleKVOContext];
    [self.webView removeObserver:self forKeyPath:@"loading" context:kEclipseLittleKVOContext];
    [self.webView removeObserver:self forKeyPath:@"title" context:kEclipseLittleKVOContext];
}

/* NSWindowDelegate */

- (void)windowWillClose:(NSNotification *)notification {
    (void)notification;
    if (self.closed) return;
    // A dialog still showing is answered (Cancel) before the page goes away.
    eclipse_end_all_sheets(self.window);
    self.closed = YES;
    [self stopObserving];
    WKWebView *webView = self.webView;
    webView.UIDelegate = nil;
    webView.navigationDelegate = nil;
    [webView stopLoading];
    // Released on the next turn of the run loop: the window is still in the
    // middle of closing.
    EclipseLittleWindow *me = self;
    dispatch_async(dispatch_get_main_queue(), ^{
        [gLittleWindows removeObject:me];
        if (eclipse_perf_enabled()) {
            fprintf(stderr, "[little] closed open=%lu\n", (unsigned long)gLittleWindows.count);
        }
        if (gLittleWindows.count == 0) {
            [gLittleOpenerTimer invalidate];
            gLittleOpenerTimer = nil;
        }
    });
}

/* WKNavigationDelegate */

- (void)webView:(WKWebView *)webView
    decidePolicyForNavigationAction:(WKNavigationAction *)action
                    decisionHandler:(void (^)(WKNavigationActionPolicy))decisionHandler {
    (void)webView;
    NSURL *url = action.request.URL;
    NSString *scheme = url.scheme.lowercaseString ?: @"";
    BOOL mainFrame = action.targetFrame == nil || action.targetFrame.isMainFrame;
    BOOL allowed = eclipse_is_web_scheme(scheme) || [scheme isEqualToString:@"about"] ||
                   [scheme isEqualToString:@"blob"] || (!mainFrame && [scheme isEqualToString:@"data"]);
    if (!allowed && mainFrame && action.navigationType == WKNavigationTypeLinkActivated &&
        ([scheme isEqualToString:@"mailto"] || [scheme isEqualToString:@"tel"])) {
        [[NSWorkspace sharedWorkspace] openURL:url];
    }
    if (!allowed && eclipse_perf_enabled()) {
        fprintf(stderr, "[little] refused %s navigation\n", scheme.UTF8String);
    }
    decisionHandler(allowed ? WKNavigationActionPolicyAllow : WKNavigationActionPolicyCancel);
}

- (void)webView:(WKWebView *)webView
    decidePolicyForNavigationResponse:(WKNavigationResponse *)response
                      decisionHandler:(void (^)(WKNavigationResponsePolicy))decisionHandler {
    (void)webView;
    // Popups do not download files.
    decisionHandler(response.canShowMIMEType ? WKNavigationResponsePolicyAllow
                                             : WKNavigationResponsePolicyCancel);
}

- (void)webView:(WKWebView *)webView didCommitNavigation:(WKNavigation *)navigation {
    (void)navigation;
    if (self.closed) return;
    self.displayURL = webView.URL;
    [self refreshTitle];
    eclipse_little_log(@"committed", self);
}

- (void)webViewWebContentProcessDidTerminate:(WKWebView *)webView {
    if (!self.closed) [webView reload];
}

/* WKUIDelegate */

- (WKWebView *)webView:(WKWebView *)webView
    createWebViewWithConfiguration:(WKWebViewConfiguration *)configuration
               forNavigationAction:(WKNavigationAction *)action
                    windowFeatures:(WKWindowFeatures *)features {
    (void)webView;
    if (self.closed) return nil;
    NSURL *url = action.request.URL;
    WKWebView *opener = self.opener;
    // A popup window.open with a user gesture nests another Little Eclipse
    // window, as it would from a tab (within the same limits).
    if (eclipse_features_want_popup(features) && opener != nil &&
        eclipse_action_user_initiated(action) && eclipse_little_initial_url_allowed(url)) {
        BOOL hasPosition = features.x != nil && features.y != nil;
        NSPoint topLeft = hasPosition ? NSMakePoint(features.x.doubleValue, features.y.doubleValue) : NSZeroPoint;
        EclipseLittleWindow *nested = eclipse_little_window_create(
            configuration, opener, NSMakeSize(features.width.doubleValue, features.height.doubleValue),
            hasPosition, topLeft);
        if (nested != nil) return nested.webView;
    }
    // Anything else (a target=_blank link, window.open without popup
    // features or a gesture, a popup over the limit): a web page opens as a
    // tab, as it would from a tab.
    if (eclipse_is_web_scheme(url.scheme) && gCallback != NULL) {
        NSString *command = [@"little_window_new_tab:" stringByAppendingString:url.absoluteString];
        gCallback(command.UTF8String);
        // A link the user clicked shows its tab; a script's tab stays
        // behind, so a page cannot push its popup out of sight.
        NSWindow *browser = gBrowserWindow;
        if (browser != nil && eclipse_action_user_initiated(action)) {
            [browser makeKeyAndOrderFront:nil];
        }
    }
    return nil;
}

- (void)webViewDidClose:(WKWebView *)webView {
    (void)webView;
    if (!self.closed) [self.window close];
}

/* WebKit's private UI-delegate methods (WKUIDelegatePrivate). */

/* popup.focus() from the opener, or a window.open that reuses this popup's
   name. WebKit only asks when the page may move focus (the opener, or a user
   gesture). */
- (void)_focusWebView:(WKWebView *)webView {
    (void)webView;
    if (!self.closed) [self.window makeKeyAndOrderFront:nil];
}

/* window.print() in the popup. */
- (void)_webView:(WKWebView *)webView printFrame:(id)frame {
    (void)frame;
    if (!self.closed) eclipse_print_webview(webView);
}

/* outerWidth/outerHeight/screenX/screenY. WebKit converts between this
   (AppKit screen coordinates, origin bottom-left) and the page's. */
- (void)_webView:(WKWebView *)webView getWindowFrameWithCompletionHandler:(void (^)(CGRect))completionHandler {
    (void)webView;
    completionHandler(self.closed ? CGRectZero : NSRectToCGRect(self.window.frame));
}

/* resizeTo/resizeBy/moveTo/moveBy (WebKit allows them only for windows a
   script opened), and the size and position window.open asked for. The
   window keeps within the limits it was opened with and on screen. */
- (void)_webView:(WKWebView *)webView setWindowFrame:(CGRect)requested {
    (void)webView;
    NSWindow *window = self.window;
    if (self.closed || window == nil || (window.styleMask & NSWindowStyleMaskFullScreen) != 0) return;
    NSRect frame = NSRectFromCGRect(requested);
    if (!isfinite(frame.origin.x) || !isfinite(frame.origin.y) || !isfinite(frame.size.width) ||
        !isfinite(frame.size.height)) {
        return;
    }
    NSScreen *target = window.screen ?: [NSScreen mainScreen];
    NSPoint center = NSMakePoint(NSMidX(frame), NSMidY(frame));
    for (NSScreen *screen in [NSScreen screens]) {
        if (NSPointInRect(center, screen.frame)) {
            target = screen;
            break;
        }
    }
    if (target == nil) return;
    frame = eclipse_little_fit_frame(window, frame, target.visibleFrame);
    [window setFrame:frame display:YES];
}

/* Dialogs are sheets on the popup. The completion handler runs exactly once:
   immediately with Cancel when the popup is not on screen or already shows a
   dialog, otherwise when the sheet ends (also when the window closes). */
- (BOOL)canPresentSheet {
    NSWindow *window = self.window;
    return !self.closed && window != nil && window.isVisible && window.attachedSheet == nil &&
           !self.webView.isHiddenOrHasHiddenAncestor;
}

- (void)webView:(WKWebView *)webView
    runJavaScriptAlertPanelWithMessage:(NSString *)message
                      initiatedByFrame:(WKFrameInfo *)frame
                     completionHandler:(void (^)(void))completionHandler {
    (void)webView;
    if (![self canPresentSheet]) {
        completionHandler();
        return;
    }
    NSAlert *alert = eclipse_dialog_alert(frame, message);
    [alert addButtonWithTitle:@"OK"];
    [alert beginSheetModalForWindow:self.window completionHandler:^(NSModalResponse response) {
        (void)response;
        completionHandler();
    }];
}

- (void)webView:(WKWebView *)webView
    runJavaScriptConfirmPanelWithMessage:(NSString *)message
                        initiatedByFrame:(WKFrameInfo *)frame
                       completionHandler:(void (^)(BOOL))completionHandler {
    (void)webView;
    if (![self canPresentSheet]) {
        completionHandler(NO);
        return;
    }
    NSAlert *alert = eclipse_dialog_alert(frame, message);
    [alert addButtonWithTitle:@"OK"];
    [alert addButtonWithTitle:@"Cancel"];
    [alert beginSheetModalForWindow:self.window completionHandler:^(NSModalResponse response) {
        completionHandler(response == NSAlertFirstButtonReturn);
    }];
}

- (void)webView:(WKWebView *)webView
    runJavaScriptTextInputPanelWithPrompt:(NSString *)prompt
                              defaultText:(NSString *)defaultText
                         initiatedByFrame:(WKFrameInfo *)frame
                        completionHandler:(void (^)(NSString *))completionHandler {
    (void)webView;
    if (![self canPresentSheet]) {
        completionHandler(nil);
        return;
    }
    NSAlert *alert = eclipse_dialog_alert(frame, prompt);
    [alert addButtonWithTitle:@"OK"];
    [alert addButtonWithTitle:@"Cancel"];
    NSTextField *field = [[NSTextField alloc] initWithFrame:NSMakeRect(0, 0, 300, 24)];
    field.stringValue = defaultText ?: @"";
    alert.accessoryView = field;
    alert.window.initialFirstResponder = field;
    [alert beginSheetModalForWindow:self.window completionHandler:^(NSModalResponse response) {
        completionHandler(response == NSAlertFirstButtonReturn ? field.stringValue : nil);
    }];
}

- (void)webView:(WKWebView *)webView
    runOpenPanelWithParameters:(WKOpenPanelParameters *)parameters
              initiatedByFrame:(WKFrameInfo *)frame
             completionHandler:(void (^)(NSArray<NSURL *> *))completionHandler {
    (void)webView; (void)frame;
    if (![self canPresentSheet]) {
        completionHandler(nil);
        return;
    }
    NSOpenPanel *panel = [NSOpenPanel openPanel];
    panel.canChooseFiles = YES;
    panel.allowsMultipleSelection = parameters.allowsMultipleSelection;
    panel.canChooseDirectories = parameters.allowsDirectories;
    [panel beginSheetModalForWindow:self.window completionHandler:^(NSModalResponse response) {
        completionHandler(response == NSModalResponseOK ? panel.URLs : nil);
    }];
}

@end

/* Whether `opener` (a tab's webview) is still showing in the browser. */
static BOOL eclipse_little_opener_alive(WKWebView *opener) {
    return opener != nil && opener.superview != nil && opener.window != nil;
}

static void eclipse_little_check_openers(void) {
    for (EclipseLittleWindow *lw in [gLittleWindows allObjects]) {
        if (!lw.closed && !eclipse_little_opener_alive(lw.opener)) {
            if (eclipse_perf_enabled()) {
                fprintf(stderr, "[little] opener tab is gone, closing\n");
            }
            [lw.window close];
        }
    }
}

static void eclipse_little_start_opener_timer(void) {
    if (gLittleOpenerTimer != nil) return;
    gLittleOpenerTimer = [NSTimer timerWithTimeInterval:0.5
                                                repeats:YES
                                                  block:^(NSTimer *timer) {
                                                      (void)timer;
                                                      eclipse_little_check_openers();
                                                  }];
    gLittleOpenerTimer.tolerance = 0.2;
    [[NSRunLoop mainRunLoop] addTimer:gLittleOpenerTimer forMode:NSRunLoopCommonModes];
}

static BOOL eclipse_frame_on_screen(NSRect frame) {
    for (NSScreen *screen in [NSScreen screens]) {
        if (NSContainsRect(screen.visibleFrame, frame)) return YES;
    }
    return NO;
}

/* Moves `frame` inside `visible` (it already fits: content is at most 90% of
   it). */
static NSRect eclipse_clamp_frame(NSRect frame, NSRect visible) {
    if (NSMaxX(frame) > NSMaxX(visible)) frame.origin.x = NSMaxX(visible) - frame.size.width;
    if (NSMinX(frame) < NSMinX(visible)) frame.origin.x = NSMinX(visible);
    if (NSMaxY(frame) > NSMaxY(visible)) frame.origin.y = NSMaxY(visible) - frame.size.height;
    if (NSMinY(frame) < NSMinY(visible)) frame.origin.y = NSMinY(visible);
    return frame;
}

/* The largest content size a popup may have on a screen whose visible frame
   is `visible`. */
static NSSize eclipse_little_max_content_size(NSRect visible) {
    return NSMakeSize(MAX(kEclipseLittleMinWidth, floor(visible.size.width * 0.9)),
                      MAX(kEclipseLittleMinHeight, floor(visible.size.height * 0.9)));
}

/* `frame` (a window frame a page asked for) with its content size within
   the popup limits, its top-left corner where it was asked for, and moved
   inside `visible`. */
static NSRect eclipse_little_fit_frame(NSWindow *window, NSRect frame, NSRect visible) {
    NSRect content = [window contentRectForFrameRect:frame];
    NSSize max = eclipse_little_max_content_size(visible);
    content.size.width = MIN(MAX(content.size.width, kEclipseLittleMinWidth), max.width);
    content.size.height = MIN(MAX(content.size.height, kEclipseLittleMinHeight), max.height);
    NSRect fitted = [window frameRectForContentRect:content];
    fitted.origin.x = frame.origin.x;
    fitted.origin.y = NSMaxY(frame) - fitted.size.height;
    return eclipse_clamp_frame(fitted, visible);
}

/* Whether one more window may open for `opener` (a tab's webview). */
static BOOL eclipse_little_has_room(WKWebView *opener) {
    NSUInteger total = 0, forOpener = 0;
    for (EclipseLittleWindow *lw in gLittleWindows) {
        if (lw.closed) continue;
        total++;
        if (lw.opener == opener) forOpener++;
    }
    return total < kEclipseLittleMaxTotal && forOpener < kEclipseLittleMaxPerTab;
}

static EclipseLittleWindow *eclipse_little_window_create(WKWebViewConfiguration *configuration,
                                                         WKWebView *opener,
                                                         NSSize requested,
                                                         BOOL hasPosition,
                                                         NSPoint topLeft) {
    if (configuration == nil || !eclipse_little_opener_alive(opener)) return nil;
    if (!gLittleGuardsReady || !eclipse_scheme_handler_guarded(configuration, @"eclipse")) {
        if (eclipse_perf_enabled()) fprintf(stderr, "[little] guards missing, opening as a tab\n");
        return nil;
    }
    if (!eclipse_little_has_room(opener)) {
        if (eclipse_perf_enabled()) fprintf(stderr, "[little] limit reached, opening as a tab\n");
        return nil;
    }

    NSWindow *anchor = opener.window ?: eclipse_browser_window();
    NSScreen *screen = anchor.screen ?: [NSScreen mainScreen];
    if (screen == nil) return nil;
    NSRect visible = screen.visibleFrame;

    NSSize maxSize = eclipse_little_max_content_size(visible);
    // A missing dimension (0) gets the default size.
    CGFloat width = isfinite(requested.width) && requested.width > 0 ? requested.width
                                                                      : kEclipseLittleDefaultWidth;
    CGFloat height = isfinite(requested.height) && requested.height > 0 ? requested.height
                                                                         : kEclipseLittleDefaultHeight;
    width = MIN(MAX(width, kEclipseLittleMinWidth), maxSize.width);
    height = MIN(MAX(height, kEclipseLittleMinHeight), maxSize.height);
    NSRect contentRect = NSMakeRect(0, 0, width, height);

    NSWindowStyleMask style = NSWindowStyleMaskTitled | NSWindowStyleMaskClosable |
                              NSWindowStyleMaskMiniaturizable | NSWindowStyleMaskResizable;
    NSWindow *window = [[NSWindow alloc] initWithContentRect:contentRect
                                                   styleMask:style
                                                     backing:NSBackingStoreBuffered
                                                       defer:NO];
    window.releasedWhenClosed = NO;
    window.restorable = NO;
    // Shown over the browser window also when that is in full screen.
    window.collectionBehavior |= NSWindowCollectionBehaviorFullScreenAuxiliary;
    window.contentMinSize = NSMakeSize(kEclipseLittleMinWidth, kEclipseLittleMinHeight);
    if ([window respondsToSelector:@selector(setTabbingMode:)]) {
        window.tabbingMode = NSWindowTabbingModeDisallowed;
    }

    WKWebView *webView = nil;
    @try {
        webView = [[WKWebView alloc] initWithFrame:contentRect configuration:configuration];
    } @catch (NSException *exception) {
        if (eclipse_perf_enabled()) {
            fprintf(stderr, "[little] webview creation failed: %s\n", exception.reason.UTF8String ?: "");
        }
        return nil;
    }
    if (webView == nil) return nil;
    // Marked before it loads anything: its script messages are never the
    // opener tab's page.
    if (gLittleWebViews == nil) gLittleWebViews = [NSHashTable weakObjectsHashTable];
    [gLittleWebViews addObject:webView];
    webView.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
    webView.customUserAgent = opener.customUserAgent;
    webView.allowsMagnification = YES;
    // respondsToSelector rather than @available: the 13.3 check is above the
    // 11.0 deployment target and would need compiler-rt, which rustc's link
    // line does not include.
    if ([webView respondsToSelector:@selector(setInspectable:)]) {
        [webView setValue:@YES forKey:@"inspectable"];
    }
    window.contentView = webView;

    // Where: the requested spot if the whole window fits on a screen there,
    // else a step down from the last popup still open, else centred over the
    // browser window.
    NSRect frame = [window frameRectForContentRect:contentRect];
    BOOL placed = NO;
    if (hasPosition && isfinite(topLeft.x) && isfinite(topLeft.y)) {
        NSScreen *primary = [NSScreen screens].firstObject;
        if (primary != nil) {
            NSRect candidate = frame;
            candidate.origin.x = topLeft.x;
            candidate.origin.y = NSMaxY(primary.frame) - topLeft.y - frame.size.height;
            if (eclipse_frame_on_screen(candidate)) {
                frame = candidate;
                placed = YES;
            }
        }
    }
    EclipseLittleWindow *previous = gLastLittleWindow;
    if (!placed && previous != nil && !previous.closed && previous.window.isVisible) {
        NSRect last = previous.window.frame;
        NSRect candidate = frame;
        candidate.origin.x = NSMinX(last) + 24.0;
        candidate.origin.y = NSMaxY(last) - 24.0 - frame.size.height;
        if (NSContainsRect(visible, candidate)) {
            frame = candidate;
            placed = YES;
        }
    }
    if (!placed) {
        NSRect around = anchor != nil ? anchor.frame : visible;
        frame.origin.x = floor(NSMidX(around) - frame.size.width / 2.0);
        frame.origin.y = floor(NSMidY(around) - frame.size.height / 2.0);
        frame = eclipse_clamp_frame(frame, visible);
    }
    [window setFrame:frame display:NO];

    EclipseLittleWindow *lw = [[EclipseLittleWindow alloc] init];
    lw.window = window;
    lw.webView = webView;
    lw.opener = opener;
    webView.UIDelegate = lw;
    webView.navigationDelegate = lw;
    window.delegate = lw;
    [webView addObserver:lw forKeyPath:@"URL" options:0 context:kEclipseLittleKVOContext];
    [webView addObserver:lw forKeyPath:@"loading" options:0 context:kEclipseLittleKVOContext];
    [webView addObserver:lw forKeyPath:@"title" options:0 context:kEclipseLittleKVOContext];
    lw.observing = YES;
    [lw refreshTitle];

    if (gLittleWindows == nil) gLittleWindows = [NSMutableSet set];
    [gLittleWindows addObject:lw];
    gLastLittleWindow = lw;
    eclipse_little_start_opener_timer();

    [window makeKeyAndOrderFront:nil];
    if (eclipse_perf_enabled()) {
        fprintf(stderr, "[little] opened %.0fx%.0f open=%lu\n", width, height,
                (unsigned long)gLittleWindows.count);
    }
    return lw;
}

/* Opens a Little Eclipse window for a popup requested by `openerPtr` (a tab's
   WKWebView), built on `configurationPtr` (the WKWebViewConfiguration WebKit
   handed the opener's UI delegate). `x`/`y` are the requested top-left in
   screen coordinates with y downwards from the primary screen's top.
   Only called from wry's new-window handler, inside the wrapped UI delegate
   method: a request without a user gesture gets no window.
   Returns the popup's WKWebView retained (+1; the caller owns that
   reference and hands it to WebKit), or NULL when no window may or could be
   made (the caller then opens the URL as a tab). */
/* Whether the window.open being decided (inside wry's new-window handler)
   asked for a popup window; see eclipse_features_want_popup. */
int eclipse_popup_request_wants_popup(void) {
    return (gPopupRequestContext & ECLIPSE_POPUP_WANTS_POPUP) != 0;
}

void *eclipse_open_little_window(void *configurationPtr, void *openerPtr, double width, double height,
                                 int hasPosition, double x, double y) {
    if (configurationPtr == NULL || openerPtr == NULL) return NULL;
    @autoreleasepool {
        WKWebViewConfiguration *configuration = (__bridge WKWebViewConfiguration *)configurationPtr;
        WKWebView *opener = (__bridge WKWebView *)openerPtr;
        if (![configuration isKindOfClass:[WKWebViewConfiguration class]] ||
            ![opener isKindOfClass:[WKWebView class]]) {
            return NULL;
        }
        if ((gPopupRequestContext & ECLIPSE_POPUP_USER_INITIATED) == 0) {
            if (eclipse_perf_enabled()) fprintf(stderr, "[little] no user gesture, opening as a tab\n");
            return NULL;
        }
        // A popup that gave only a width or a height keeps it (wry reports
        // a size only when both are given; the other gets the default).
        WKWindowFeatures *features = gPopupRequestFeatures;
        if (width <= 0 && features.width != nil) width = features.width.doubleValue;
        if (height <= 0 && features.height != nil) height = features.height.doubleValue;
        EclipseLittleWindow *lw = eclipse_little_window_create(
            configuration, opener, NSMakeSize(width, height), hasPosition != 0, NSMakePoint(x, y));
        if (lw == nil) return NULL;
        return (__bridge_retained void *)lw.webView;
    }
}

static EclipseLittleWindow *eclipse_little_window_for(NSWindow *window) {
    if (window == nil) return nil;
    for (EclipseLittleWindow *lw in gLittleWindows) {
        if (!lw.closed && lw.window == window) return lw;
    }
    return nil;
}

static BOOL eclipse_is_little_window(NSWindow *window) {
    return eclipse_little_window_for(window) != nil;
}

/* Menu commands while a Little Eclipse window is key (or one of its sheets
   is). Page commands act on the popup; commands about a tab's page (Home,
   Bookmark, Find, which needs the tab's in-page find bar) are refused rather
   than sent to the hidden tab behind the popup. YES when the command was
   handled here; the rest go to the browser (⌘W closes the popup, see
   performEclipseAction:). */
static BOOL eclipse_little_window_perform_action(NSWindow *window, NSString *action) {
    EclipseLittleWindow *lw = eclipse_little_window_for(window);
    if (lw == nil) return NO;
    WKWebView *webView = lw.webView;
    // While a dialog is up its page is paused: page commands wait.
    BOOL dialog = lw.window.attachedSheet != nil;
    static NSSet<NSString *> *pageCommands = nil;
    static NSSet<NSString *> *tabCommands = nil;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        pageCommands = [NSSet setWithArray:@[ @"reload", @"stop_loading", @"go_back", @"go_forward", @"print",
                                              @"zoom_in", @"zoom_out", @"zoom_reset", @"open_devtools" ]];
        tabCommands = [NSSet setWithArray:@[ @"home", @"favorite_tab", @"duplicate_tab", @"find", @"find_next",
                                             @"find_prev" ]];
    });
    if ([tabCommands containsObject:action] || (dialog && [pageCommands containsObject:action])) {
        NSBeep();
        return YES;
    }
    if (![pageCommands containsObject:action]) return NO;

    if ([action isEqualToString:@"reload"]) {
        [webView reload];
    } else if ([action isEqualToString:@"stop_loading"]) {
        [webView stopLoading];
    } else if ([action isEqualToString:@"go_back"]) {
        [webView goBack];
    } else if ([action isEqualToString:@"go_forward"]) {
        [webView goForward];
    } else if ([action isEqualToString:@"print"]) {
        eclipse_print_webview(webView);
    } else if ([action hasPrefix:@"zoom_"]) {
        if (@available(macOS 11.0, *)) {
            // The same steps and limits as a tab (tab::MIN_ZOOM/MAX_ZOOM).
            double zoom = webView.pageZoom;
            if ([action isEqualToString:@"zoom_in"]) zoom += 0.1;
            else if ([action isEqualToString:@"zoom_out"]) zoom -= 0.1;
            else zoom = 1.0;
            webView.pageZoom = MIN(MAX(round(zoom * 100.0) / 100.0, 0.5), 3.0);
        }
    } else if ([action isEqualToString:@"open_devtools"]) {
        // The popup's own Web Inspector (WebKit's _inspector SPI).
        SEL inspectorSel = NSSelectorFromString(@"_inspector");
        id inspector = [webView respondsToSelector:inspectorSel]
                           ? ((id (*)(id, SEL))objc_msgSend)(webView, inspectorSel)
                           : nil;
        SEL showSel = NSSelectorFromString(@"show");
        if ([inspector respondsToSelector:showSel]) {
            ((void (*)(id, SEL))objc_msgSend)(inspector, showSel);
        } else {
            NSBeep();
        }
    }
    return YES;
}

/* How many Little Eclipse windows are open for the tab whose webview is (or
   contains) `viewPtr`, nested popups included. Rust keeps such a tab awake:
   hibernating it would close them. */
int eclipse_little_window_count_for_view(void *viewPtr) {
    if (viewPtr == NULL || gLittleWindows.count == 0) return 0;
    @autoreleasepool {
        WKWebView *webView = eclipse_find_wkwebview((__bridge NSView *)viewPtr);
        if (webView == nil) return 0;
        int count = 0;
        for (EclipseLittleWindow *lw in gLittleWindows) {
            if (!lw.closed && lw.opener == webView) count++;
        }
        return count;
    }
}
