#import <Cocoa/Cocoa.h>
#import <QuartzCore/QuartzCore.h>
#import <WebKit/WebKit.h>
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

    // ⌘W / ⇧⌘W in a secondary window (detached Web Inspector, About panel)
    // closes that window instead of a browser tab behind it.
    if ([action isEqualToString:@"close_tab"] || [action isEqualToString:@"close_window"]) {
        NSWindow *key = [NSApp keyWindow];
        NSWindow *browser = gBrowserWindow;
        if (key != nil && browser != nil && key != browser &&
            (key.styleMask & NSWindowStyleMaskClosable) != 0) {
            [key performClose:nil];
            return;
        }
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
   report to Rust with the tab id registered for that webview. */

typedef void (*EclipseNavigationCallback)(uint64_t tabId, int kind, int64_t code,
                                          const char *description, const char *url);

static EclipseNavigationCallback gNavigationCallback = NULL;
static NSMapTable<WKWebView *, NSNumber *> *gTabIdsByWebView = nil;

static void eclipse_report_navigation_failure(WKWebView *webView, int kind, NSError *error) {
    if (gNavigationCallback == NULL || webView == nil || error == nil) return;
    NSNumber *tabId = [gTabIdsByWebView objectForKey:webView];
    if (tabId == nil) return;

    // Superseded by another navigation.
    if ([error.domain isEqualToString:NSURLErrorDomain] && error.code == NSURLErrorCancelled) return;
    // WebKitErrorFrameLoadInterruptedByPolicyChange: the response became a
    // download or the navigation handler cancelled it.
    if ([error.domain isEqualToString:@"WebKitErrorDomain"] && error.code == 102) return;
    // A crashed WebContent process is reported by the terminate handler.
    if ([error.domain isEqualToString:WKErrorDomain] && error.code == WKErrorWebContentProcessTerminated) return;

    NSURL *failingURL = error.userInfo[NSURLErrorFailingURLErrorKey];
    NSString *url = [failingURL isKindOfClass:[NSURL class]] ? failingURL.absoluteString : nil;
    if (url.length == 0) {
        NSString *failingString = error.userInfo[NSURLErrorFailingURLStringErrorKey];
        url = [failingString isKindOfClass:[NSString class]] ? failingString : nil;
    }
    if (url.length == 0) {
        url = webView.URL.absoluteString;
    }
    // Links to schemes WebKit cannot load (mailto:, app links) must not
    // replace the current page with an error page.
    NSString *scheme = [[NSURL URLWithString:url ?: @""] scheme].lowercaseString;
    if (scheme.length > 0 && ![scheme isEqualToString:@"http"] && ![scheme isEqualToString:@"https"] &&
        ![scheme isEqualToString:@"file"]) {
        return;
    }

    NSString *description = error.localizedDescription ?: @"";
    gNavigationCallback(tabId.unsignedLongLongValue, kind, (int64_t)error.code,
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
   exactly once, also when the sheet is dismissed because its window closes. */
static void eclipse_present_dialog(WKWebView *webView, NSAlert *alert, void (^done)(NSModalResponse)) {
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

/* WebKit's private UI-delegate callback for window.print(). WebKit resumes
   the page once this returns, so the print sheet starts on the next turn of
   the run loop, when the page can render for it. */
static void eclipse_print_frame(id self, SEL _cmd, WKWebView *webView, id frame) {
    (void)self; (void)_cmd; (void)frame;
    __weak WKWebView *weakWebView = webView;
    dispatch_async(dispatch_get_main_queue(), ^{
        WKWebView *wv = weakWebView;
        NSWindow *window = wv.window;
        if (wv == nil || window == nil) return;
        if (![wv respondsToSelector:@selector(printOperationWithPrintInfo:)]) return;
        NSPrintOperation *operation = [wv printOperationWithPrintInfo:[NSPrintInfo sharedPrintInfo]];
        operation.canSpawnSeparateThread = YES;
        [operation runOperationModalForWindow:window delegate:nil didRunSelector:NULL contextInfo:NULL];
    });
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
    }
}
