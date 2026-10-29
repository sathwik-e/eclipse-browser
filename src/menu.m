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
static BOOL eclipse_is_command_bar_window(NSWindow *window);
static NSWindow *eclipse_command_bar_dismiss_for_command(void);

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

    // The ⌥⌘N bar in front: ⌥⌘N again refreshes it; any other command closes
    // it and is for the window that was in front before it (⌘W only closes
    // the bar).
    if (eclipse_is_command_bar_window(key) && ![action isEqualToString:@"new_little_window"]) {
        key = eclipse_command_bar_dismiss_for_command();
        if ([action isEqualToString:@"close_tab"] || [action isEqualToString:@"close_window"]) return;
    }

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
    // A new Little Eclipse window opens over whatever is in front.
    if (browser != nil && key != browser && eclipse_is_little_window(key) &&
        ![action isEqualToString:@"new_little_window"]) {
        [browser makeKeyAndOrderFront:nil];
    }
    gCallback(action.UTF8String);
}

/* "Open in Eclipse" (⌘O) is only for a Little Eclipse window in front. */
- (BOOL)validateMenuItem:(NSMenuItem *)item {
    id action = item.representedObject;
    if ([action isKindOfClass:[NSString class]] && [action isEqualToString:@"open_in_eclipse"]) {
        return eclipse_is_little_window(eclipse_shortcut_window());
    }
    return YES;
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
        EclipseAddItem(fileMenu, @"New Little Eclipse Window", @selector(performEclipseAction:), @"n",
                       NSEventModifierFlagCommand | NSEventModifierFlagOption, gMenuTarget,
                       @"new_little_window");
        EclipseAddItem(fileMenu, @"Open in Eclipse", @selector(performEclipseAction:), @"o",
                       NSEventModifierFlagCommand, gMenuTarget, @"open_in_eclipse");
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

/* Gives `wv` the rule list now, or once it is compiled. */
static void eclipse_adblock_want_webview(WKWebView *wv) {
    if (wv == nil) return;
    if (gAdblockWantedWebViews == nil) {
        gAdblockWantedWebViews = [NSHashTable weakObjectsHashTable];
        gAdblockAttachedWebViews = [NSHashTable weakObjectsHashTable];
    }
    [gAdblockWantedWebViews addObject:wv];
    if (gEclipseContentRuleList != nil) {
        eclipse_attach_rule_list(wv);
    } else {
        eclipse_compile_content_rule_list_if_needed();
    }
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
            eclipse_adblock_want_webview(wv);
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
   bar. */
static BOOL eclipse_features_want_popup(WKWindowFeatures *features) {
    if (features == nil) return NO;
    SEL sel = NSSelectorFromString(@"_wantsPopup");
    if ([features respondsToSelector:sel]) return ((BOOL (*)(id, SEL))objc_msgSend)(features, sel);
    // Older WebKit: any size or position, or toolbars explicitly off.
    if (features.width != nil || features.height != nil || features.x != nil || features.y != nil) return YES;
    return features.toolbarsVisibility != nil && !features.toolbarsVisibility.boolValue;
}

/* Whether a new window opens as a Little Eclipse window (which keeps
   window.opener) rather than a tab, as in Arc: a window.open that asked for a
   popup window (above), and also one with no features at all
   (window.open(url, name)) as long as the new page gets an opener. WebKit
   keeps the opener only for a page in a web view of its own, so such a
   window cannot be a tab. A link the user clicked (target=_blank, which
   WebKit opens without an opener anyway), a form, and window.open(url,
   '_blank', 'noopener') are tabs. -[WKNavigationAction _hasOpener] is SPI
   (present since macOS 10.15); without it only popup features count. */
static BOOL eclipse_request_wants_popup(WKNavigationAction *action, WKWindowFeatures *features) {
    if (eclipse_features_want_popup(features)) return YES;
    if (action == nil || action.navigationType != WKNavigationTypeOther) return NO;
    SEL sel = NSSelectorFromString(@"_hasOpener");
    if (![action respondsToSelector:sel]) return NO;
    return ((BOOL (*)(id, SEL))objc_msgSend)(action, sel);
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
                           (eclipse_request_wants_popup(action, features) ? ECLIPSE_POPUP_WANTS_POPUP : 0);
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
/* Small native windows in the manner of Arc's "Little Arc": no sidebar and no
   tabs, a slim top bar framed in the current Space's theme colour, and the page
   as a rounded card below it. There are two kinds.

   Popups. Sign-in and payment popups (window.open with popup features, from a
   web page) open in a Little Eclipse window instead of a tab, so they keep
   window.opener and can post their result back. WebKit requires the popup's
   WKWebView to be built on the configuration it hands the opener's UI
   delegate; that configuration shares the opener tab's
   WKUserContentController, so the popup runs the tab's init scripts and its
   messages reach the tab's IPC handler (Rust only trusts them as an ordinary
   website). A popup opens as a window only for a window.open with a user
   gesture, and at most kEclipseLittleMaxPerTab per tab and
   kEclipseLittleMaxTotal in all; anything else opens as a tab. The guards
   above keep popups away from wry's eclipse:// handler and mark their script
   messages. Each popup follows its root opener (the tab's webview): when that
   webview leaves its window (the tab was closed, hibernated or rebuilt) the
   popup closes.

   Standalone windows. Links from other apps (when Eclipse is the default
   browser) and File > New Little Eclipse Window open one without an opener,
   built on a fresh configuration that shares only the tabs' website data
   store (cookies and sign-ins), the user agent and the content blocker Rust
   passes in: no tab scripts and no IPC handler. They count toward the overall
   limit only and never close because of an opener. A popup a standalone
   window opens follows that window as its root opener.

   The top bar, like Arc's, has a faint diagonal-stripe texture over the
   Space's colour; on its left a site-information button (a popover with the
   connection's security, the host, Copy Link and Open in Eclipse) and the
   full address of the document on screen in a small monospaced font, the
   host brighter than the rest; on its right Copy Link and Open in Eclipse.
   The address is derived here from WebKit's URL and never from page content:
   that of a load only once it commits, never of one still pending. When it
   does not fit, the path is shortened first and the host last, and "Not
   Secure" for anything but https always stays. Clicking the address shows it
   for editing; Return hands what was typed to Rust, which resolves it to a
   web address or a search and loads it through eclipse_little_window_load
   (http(s) only). "Open in Eclipse" (⌘O) moves the page into a new tab of
   the browser window. A Little Eclipse window only ever loads web content:
   internal eclipse:// pages and other schemes are refused by its navigation
   delegate. The window title is the page title, for Mission Control and the
   Window menu; the address on the bar is never page-controlled. */

@class EclipseLittleAddress;

@interface EclipseLittleWindow : NSObject <NSWindowDelegate, WKUIDelegate, WKNavigationDelegate, NSTextFieldDelegate>
@property(nonatomic, strong) NSWindow *window;
@property(nonatomic, strong) WKWebView *webView;
/* The webview at the root of this window's popup chain: the tab webview the
   first popup was opened from, or a standalone window's own webview. nil for
   a standalone window. wry keeps a destroyed tab's WKWebView alive, so its
   detachment, not its deallocation, is what signals that the tab is gone. */
@property(nonatomic, weak) WKWebView *opener;
/* The URL of the document on screen, whose host the top bar shows: nil
   until WebKit confirms one (see -syncDisplayURL). */
@property(nonatomic, copy) NSURL *displayURL;
@property(nonatomic) BOOL observing;
@property(nonatomic) BOOL closed;
/* Opened without an opener (a link from another app, ⌥⌘N). */
@property(nonatomic) BOOL standalone;
/* Identifies the window to Rust (address-field requests). Never reused. */
@property(nonatomic) uint64_t serial;
@property(nonatomic, strong) NSView *container;
/* The faint stripes behind the bar's controls. */
@property(nonatomic, strong) NSView *texture;
/* The site-information button left of the address. */
@property(nonatomic, strong) NSButton *infoButton;
/* The address (its identifier is still "eclipse-little-host"). */
@property(nonatomic, strong) NSButton *hostButton;
/* What the address button stands for (eclipse_little_address); its title
   is this, shortened to fit (see -fitHostLabel). */
@property(nonatomic, strong) EclipseLittleAddress *address;
@property(nonatomic, strong) NSTextField *addressField;
/* The rounded background the address field sits on while editing. */
@property(nonatomic, strong) NSView *addressBox;
@property(nonatomic, strong) NSButton *linkButton;
/* "Copied", shown for a moment after Copy Link. */
@property(nonatomic, strong) NSTextField *copiedLabel;
@property(nonatomic) NSUInteger copiedGeneration;
@property(nonatomic, strong) NSView *separator;
@property(nonatomic, strong) NSButton *openButton;
@property(nonatomic, strong) NSPopover *infoPopover;
@property(nonatomic) BOOL editing;
- (void)layoutChrome;
- (void)applyTheme;
- (void)fitHostLabel;
- (void)beginEditingAddress;
- (void)openInEclipse;
- (void)copyLink;
- (void)downloadStarted;
@end

/* The bar's diagonal stripes, drawn faintly in the frame's own contrast
   colour. Never takes a click (the bar moves the window). */
@interface EclipseLittleBarTexture : NSView
@end

/* The window's content view: the theme-coloured frame that holds the bar's
   controls and the page card. Lays them out whenever its size changes. */
@interface EclipseLittleContentView : NSView
@property(nonatomic, weak) EclipseLittleWindow *owner;
@end

@implementation EclipseLittleContentView
- (BOOL)isFlipped {
    return YES;
}
- (BOOL)mouseDownCanMoveWindow {
    return YES;
}
- (void)setFrameSize:(NSSize)newSize {
    [super setFrameSize:newSize];
    [self.owner layoutChrome];
}
@end

static BOOL gLittleFrameDark;

@implementation EclipseLittleBarTexture
- (BOOL)isFlipped {
    return YES;
}
- (NSView *)hitTest:(NSPoint)point {
    (void)point;
    return nil;
}
- (void)drawRect:(NSRect)dirtyRect {
    (void)dirtyRect;
    NSRect bounds = self.bounds;
    NSColor *stripe = gLittleFrameDark ? [NSColor colorWithCalibratedWhite:1.0 alpha:0.035]
                                       : [NSColor colorWithCalibratedWhite:0.0 alpha:0.035];
    [stripe setStroke];
    NSBezierPath *path = [NSBezierPath bezierPath];
    path.lineWidth = 3.0;
    CGFloat height = NSHeight(bounds), spacing = 9.0;
    // Lines at 45°, from the bottom edge up to the right.
    for (CGFloat x = -height; x < NSWidth(bounds) + height; x += spacing) {
        [path moveToPoint:NSMakePoint(x, height)];
        [path lineToPoint:NSMakePoint(x + height, 0.0)];
    }
    [path stroke];
}
@end

static NSMutableSet<EclipseLittleWindow *> *gLittleWindows = nil;
static __weak EclipseLittleWindow *gLastLittleWindow = nil;
static NSTimer *gLittleOpenerTimer = nil;
static void *kEclipseLittleKVOContext = &kEclipseLittleKVOContext;
static uint64_t gLittleNextSerial = 1;

static const NSUInteger kEclipseLittleMaxPerTab = 3;
static const NSUInteger kEclipseLittleMaxTotal = 10;
/* Limits and defaults for the page area (what window.open's size means). */
static const CGFloat kEclipseLittleMinWidth = 360.0;
static const CGFloat kEclipseLittleMinHeight = 260.0;
/* For a popup that gave no width or height ("popup", or position only), and
   for a standalone window until the user resizes one. */
static const CGFloat kEclipseLittleDefaultWidth = 500.0;
static const CGFloat kEclipseLittleDefaultHeight = 640.0;
static const NSUInteger kEclipseLittleMaxHostLength = 64;
static const NSUInteger kEclipseLittleMaxAddressLength = 8192;
/* The frame around the page card: the top bar, and a margin on the other
   three sides. The window is the page area plus this chrome. */
static const CGFloat kEclipseLittleBarHeight = 36.0;
static const CGFloat kEclipseLittleInset = 6.0;
static const CGFloat kEclipseLittleChromeWidth = 2.0 * 6.0;          /* 2 × inset */
static const CGFloat kEclipseLittleChromeHeight = 36.0 + 6.0;        /* bar + inset */
static const CGFloat kEclipseLittleCardRadius = 10.0;

/* The current Space's theme colour (eclipse_set_little_window_theme). */
static NSColor *gLittleFrameColor = nil;
static BOOL gLittleFrameDark = YES;
/* The page-area size the user last gave a standalone window (zero: none). */
static NSSize gLittleStandaloneSize = {0.0, 0.0};

static NSColor *eclipse_little_frame_color(void) {
    return gLittleFrameColor ?: [NSColor colorWithSRGBRed:0.063 green:0.071 blue:0.094 alpha:1.0];
}

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

/* An address the browser itself may load into a Little Eclipse window (a link
   from another app, the address field): an http(s) URL with a host. */
static NSURL *eclipse_little_loadable_url(const char *text) {
    if (text == NULL) return nil;
    NSString *string = [NSString stringWithUTF8String:text];
    if (string.length == 0 || string.length > kEclipseLittleMaxAddressLength) return nil;
    NSURL *url = [NSURL URLWithString:string];
    if (url == nil || !eclipse_is_web_scheme(url.scheme) || url.host.length == 0) return nil;
    return url;
}

static BOOL eclipse_same_web_origin(NSURL *a, NSURL *b) {
    if (a == nil || b == nil) return NO;
    if (!eclipse_is_web_scheme(a.scheme) || !eclipse_is_web_scheme(b.scheme)) return NO;
    if ([a.scheme caseInsensitiveCompare:b.scheme] != NSOrderedSame) return NO;
    if (a.host == nil || b.host == nil || [a.host caseInsensitiveCompare:b.host] != NSOrderedSame) return NO;
    NSNumber *pa = a.port, *pb = b.port;
    return (pa == nil && pb == nil) || (pa != nil && pb != nil && [pa isEqualToNumber:pb]);
}

/* What the top bar shows for the committed document, in parts: the full
   address as Arc shows it (scheme, host, path and query; never the user
   name or password a URL may carry, nor the fragment), with "Not Secure — "
   in front of anything that is not https. A long host keeps its end (the
   registrable domain) and loses its start. Only web addresses show more
   than their scheme: a data: or file: URL never shows its content. */
typedef NS_ENUM(NSInteger, EclipseLittleSecurity) {
    EclipseLittleSecurityNone,  /* no page (about:) */
    EclipseLittleSecuritySecure,
    EclipseLittleSecurityNotSecure,
};

static NSString *const kEclipseLittleNotSecurePrefix = @"Not Secure — ";
static const NSUInteger kEclipseLittleMaxRestLength = 1024;

@interface EclipseLittleAddress : NSObject
@property(nonatomic) EclipseLittleSecurity security;
/* kEclipseLittleNotSecurePrefix or empty. */
@property(nonatomic, copy) NSString *prefix;
/* "https://", "http://", "blob:https://" or empty. */
@property(nonatomic, copy) NSString *scheme;
/* The host and port (or "about:blank", "data:", ...): never empty. */
@property(nonatomic, copy) NSString *host;
/* The path and query, or empty. */
@property(nonatomic, copy) NSString *rest;
/* prefix + scheme + host + rest. */
@property(nonatomic, readonly) NSString *plain;
@end

@implementation EclipseLittleAddress
- (instancetype)init {
    if ((self = [super init])) {
        _prefix = @"";
        _scheme = @"";
        _host = @"about:blank";
        _rest = @"";
    }
    return self;
}
- (NSString *)plain {
    return [NSString stringWithFormat:@"%@%@%@%@", self.prefix, self.scheme, self.host, self.rest];
}
- (BOOL)isEqual:(id)other {
    if (![other isKindOfClass:[EclipseLittleAddress class]]) return NO;
    EclipseLittleAddress *o = other;
    return o.security == self.security && [o.plain isEqualToString:self.plain] && [o.host isEqualToString:self.host];
}
- (NSUInteger)hash {
    return self.plain.hash;
}
@end

static EclipseLittleAddress *eclipse_little_address(NSURL *url) {
    EclipseLittleAddress *address = [[EclipseLittleAddress alloc] init];
    NSString *scheme = url.scheme.lowercaseString ?: @"";
    if (url == nil || scheme.length == 0) return address;
    if ([scheme isEqualToString:@"about"]) {
        // Never the query or fragment: the page controls those.
        NSString *rest = url.resourceSpecifier ?: @"";
        NSRange cut = [rest rangeOfCharacterFromSet:[NSCharacterSet characterSetWithCharactersInString:@"?#"]];
        if (cut.location != NSNotFound) rest = [rest substringToIndex:cut.location];
        if (rest.length > 64) rest = [[rest substringToIndex:64] stringByAppendingString:@"…"];
        address.host = [@"about:" stringByAppendingString:(rest.length > 0 ? rest : @"blank")];
        return address;
    }
    address.security = EclipseLittleSecurityNotSecure;
    address.prefix = kEclipseLittleNotSecurePrefix;
    if ([scheme isEqualToString:@"blob"]) {
        NSURL *inner = [NSURL URLWithString:url.resourceSpecifier ?: @""];
        if (inner != nil && eclipse_is_web_scheme(inner.scheme) && inner.host.length > 0) {
            EclipseLittleAddress *innerAddress = eclipse_little_address(inner);
            innerAddress.scheme = [@"blob:" stringByAppendingString:innerAddress.scheme];
            return innerAddress;
        }
        address.host = @"blob:";
        return address;
    }
    NSString *host = url.host.lowercaseString ?: @"";
    if (!eclipse_is_web_scheme(scheme) || host.length == 0) {
        address.host = [scheme stringByAppendingString:@":"];
        return address;
    }
    if ([host containsString:@":"]) host = [NSString stringWithFormat:@"[%@]", host];
    if (url.port != nil) host = [NSString stringWithFormat:@"%@:%@", host, url.port];
    if (host.length > kEclipseLittleMaxHostLength) {
        host = [@"…" stringByAppendingString:[host substringFromIndex:host.length - kEclipseLittleMaxHostLength]];
    }
    // Percent-encoded, so the path shows no look-alike or invisible characters.
    NSURLComponents *components = [NSURLComponents componentsWithURL:url resolvingAgainstBaseURL:YES];
    NSString *rest = components.percentEncodedPath ?: @"";
    if (components.percentEncodedQuery != nil) rest = [NSString stringWithFormat:@"%@?%@", rest, components.percentEncodedQuery];
    if (rest.length > kEclipseLittleMaxRestLength) {
        rest = [[rest substringToIndex:kEclipseLittleMaxRestLength] stringByAppendingString:@"…"];
    }
    address.host = host;
    address.scheme = [scheme stringByAppendingString:@"://"];
    address.rest = rest;
    if ([scheme isEqualToString:@"https"]) {
        address.security = EclipseLittleSecuritySecure;
        address.prefix = @"";
    }
    return address;
}

/* The security state and host alone ("Not Secure — example.com"), for the
   window title of a page without one and for logs. */
static NSString *eclipse_little_host_label(NSURL *url) {
    EclipseLittleAddress *address = eclipse_little_address(url);
    return [address.prefix stringByAppendingString:address.host];
}

/* The site-information button's symbol for a security state. */
static NSImage *eclipse_little_security_image(EclipseLittleSecurity security) {
    if (@available(macOS 11.0, *)) {
        NSArray<NSString *> *names = @[ @"info", @"info.circle" ];
        NSString *description = @"Site information";
        if (security == EclipseLittleSecurityNotSecure) {
            names = @[ @"exclamationmark.triangle.fill" ];
            description = @"Not secure: site information";
        }
        NSImageSymbolConfiguration *config = [NSImageSymbolConfiguration configurationWithPointSize:11.5
                                                                                           weight:NSFontWeightBold];
        for (NSString *name in names) {
            NSImage *image = [NSImage imageWithSystemSymbolName:name accessibilityDescription:description];
            if (image != nil) return [image imageWithSymbolConfiguration:config] ?: image;
        }
    }
    return nil;
}

static NSFont *eclipse_little_address_font(NSFontWeight weight) {
    if (@available(macOS 10.15, *)) return [NSFont monospacedSystemFontOfSize:11.0 weight:weight];
    return [NSFont userFixedPitchFontOfSize:11.0];
}

/* The first of `names` this macOS has as an SF Symbol, sized for the bar. */
static NSImage *eclipse_little_symbol(NSArray<NSString *> *names, NSString *description) {
    if (@available(macOS 11.0, *)) {
        NSImageSymbolConfiguration *config = [NSImageSymbolConfiguration configurationWithPointSize:12.0
                                                                                           weight:NSFontWeightMedium];
        for (NSString *name in names) {
            NSImage *image = [NSImage imageWithSystemSymbolName:name accessibilityDescription:description];
            if (image != nil) return [image imageWithSymbolConfiguration:config] ?: image;
        }
    }
    return nil;
}

/* The pasteboard Copy Link writes to: the general one, or for the
   end-to-end checks (ECLIPSE_TEST_PASTEBOARD) one of that name, so a test
   run leaves the user's clipboard alone. */
static NSPasteboard *eclipse_pasteboard(void) {
    const char *name = getenv("ECLIPSE_TEST_PASTEBOARD");
    if (name != NULL && name[0] != '\0') return [NSPasteboard pasteboardWithName:@(name)];
    return [NSPasteboard generalPasteboard];
}

/* The address as the bar draws it: the host bright, the scheme and path
   dimmer, "Not Secure" in orange. `scheme`, `host` and `rest` are the parts
   to show (possibly shortened). Colours are dynamic, so the title follows
   the window's appearance. */
static NSAttributedString *eclipse_little_address_title(EclipseLittleAddress *address, NSString *scheme,
                                                         NSString *host, NSString *rest) {
    NSMutableParagraphStyle *style = [[NSMutableParagraphStyle alloc] init];
    style.alignment = NSTextAlignmentLeft;
    // A last resort (see eclipse_little_fit_address): the cell cuts the end
    // rather than the security state, and otherwise the start of the host.
    style.lineBreakMode = address.prefix.length > 0 ? NSLineBreakByTruncatingTail : NSLineBreakByTruncatingHead;
    NSMutableAttributedString *title = [[NSMutableAttributedString alloc] init];
    void (^add)(NSString *, NSFont *, NSColor *) = ^(NSString *text, NSFont *font, NSColor *color) {
        if (text.length == 0) return;
        [title appendAttributedString:[[NSAttributedString alloc]
                                          initWithString:text
                                              attributes:@{NSFontAttributeName : font,
                                                           NSForegroundColorAttributeName : color,
                                                           NSParagraphStyleAttributeName : style}]];
    };
    add(address.prefix, [NSFont systemFontOfSize:11.0 weight:NSFontWeightSemibold], [NSColor systemOrangeColor]);
    add(scheme, eclipse_little_address_font(NSFontWeightRegular), [NSColor tertiaryLabelColor]);
    add(host, eclipse_little_address_font(NSFontWeightSemibold), [NSColor labelColor]);
    add(rest, eclipse_little_address_font(NSFontWeightRegular), [NSColor secondaryLabelColor]);
    return title;
}

/* `address` as it fits on `button`. When the whole of it does not, the path
   goes first (from its end, down to "/…"), then the scheme, then the rest of
   the path, and only then does the host lose characters from its start (its
   end, the registrable domain, stays). A "Not Secure — " in front always
   stays: the security state is never what gets cut. */
static NSAttributedString *eclipse_little_fit_address(NSButton *button, EclipseLittleAddress *address) {
    NSString *scheme = address.scheme, *host = address.host, *rest = address.rest;
    NSAttributedString *whole = eclipse_little_address_title(address, scheme, host, rest);
    CGFloat width = NSWidth(button.frame);
    if (!isfinite(width) || width <= 0) return whole;
    NSButtonCell *probe = [button.cell copy];
    BOOL (^fits)(NSAttributedString *) = ^BOOL(NSAttributedString *title) {
        probe.attributedTitle = title;
        return probe.cellSize.width <= width;
    };
    if (fits(whole)) return whole;
    // The longest `keep` in [low, high] for which `make(keep)` fits, or 0.
    NSUInteger (^longest)(NSUInteger, NSUInteger, NSAttributedString * (^)(NSUInteger)) =
        ^NSUInteger(NSUInteger low, NSUInteger high, NSAttributedString * (^make)(NSUInteger)) {
            NSUInteger best = 0;
            while (low <= high && high > 0) {
                NSUInteger mid = low + (high - low) / 2;
                if (fits(make(mid))) {
                    best = mid;
                    low = mid + 1;
                } else {
                    high = mid - 1;
                }
            }
            return best;
        };
    // 1. The path loses its end ("/very/long/pa…"), down to "/…".
    if (rest.length > 1) {
        NSAttributedString * (^cutRest)(NSUInteger) = ^NSAttributedString *(NSUInteger keep) {
            NSUInteger end = NSMaxRange([rest rangeOfComposedCharacterSequenceAtIndex:keep - 1]);
            NSString *shown = [[rest substringToIndex:MIN(end, rest.length)] stringByAppendingString:@"…"];
            return eclipse_little_address_title(address, scheme, host, shown);
        };
        NSUInteger keep = longest(1, rest.length - 1, cutRest);
        if (keep > 0) return cutRest(keep);
    }
    NSString *minimalRest = rest.length > 1 ? [[rest substringToIndex:1] stringByAppendingString:@"…"] : rest;
    // 2. Without the scheme, then 3. without the path.
    NSAttributedString *title = eclipse_little_address_title(address, @"", host, minimalRest);
    if (fits(title)) return title;
    title = eclipse_little_address_title(address, @"", host, @"");
    if (fits(title)) return title;
    // 4. The longest end of the host that fits after "…" (at least one
    // character).
    NSString *bare = [host hasPrefix:@"…"] ? [host substringFromIndex:1] : host;
    if (bare.length == 0) return title;
    NSAttributedString * (^cutHost)(NSUInteger) = ^NSAttributedString *(NSUInteger keep) {
        NSUInteger start = [bare rangeOfComposedCharacterSequenceAtIndex:bare.length - keep].location;
        return eclipse_little_address_title(address, @"", [@"…" stringByAppendingString:[bare substringFromIndex:start]],
                                            @"");
    };
    return cutHost(MAX((NSUInteger)1, longest(1, bare.length, cutHost)));
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
    fprintf(stderr, "[little] %s host=%s standalone=%d open=%lu\n", what.UTF8String, label.UTF8String,
            lw.standalone ? 1 : 0, (unsigned long)gLittleWindows.count);
}

/* Where the traffic lights sit: the vertical centre of their row (from the
   top of the window) and where the bar's own controls may start. AppKit
   places the buttons; they are never moved. */
static void eclipse_little_bar_metrics(NSWindow *window, CGFloat *rowMid, CGFloat *leading) {
    *rowMid = kEclipseLittleBarHeight / 2.0;
    *leading = 12.0;
    if (window == nil || (window.styleMask & NSWindowStyleMaskFullScreen) != 0) return;
    NSButton *last = [window standardWindowButton:NSWindowZoomButton] ?: [window standardWindowButton:NSWindowCloseButton];
    if (last == nil || last.superview == nil || last.isHidden) return;
    NSRect rect = [last convertRect:last.bounds toView:nil];
    CGFloat mid = NSHeight(window.frame) - NSMidY(rect);
    if (isfinite(mid) && mid > 6.0 && mid < kEclipseLittleBarHeight - 6.0) *rowMid = mid;
    if (isfinite(NSMaxX(rect)) && NSMaxX(rect) > 0) *leading = NSMaxX(rect) + 14.0;
}

static EclipseLittleWindow *eclipse_little_window_create(WKWebViewConfiguration *configuration,
                                                         WKWebView *opener,
                                                         NSSize requested,
                                                         BOOL hasPosition,
                                                         NSPoint topLeft,
                                                         BOOL standalone);

static void eclipse_little_check_openers(void);
static NSRect eclipse_little_fit_frame(NSWindow *window, NSRect frame, NSRect visible);

/* -- downloads

   A Little Eclipse window downloads what it cannot show, and what a link
   marks with `download`, as a tab does: the WKDownload goes to the
   navigation delegate of a tab's webview (wry's), which gives it wry's
   download delegate, so tab.rs names the file, saves it to ~/Downloads and
   lists it under Downloads. An EclipseLittleDownload sits in between: it
   forwards everything to that delegate (and keeps it alive should the tab
   close mid-download) and tells the window once the file has a destination.
   A window that has shown no page then closes: it was opened for the file (a
   link from another app, a popup) and would otherwise stay empty. The
   download carries on without it.
   WKDownload needs macOS 11.3, above the deployment target. Without it, or
   with no tab to take the download, a file the window cannot show opens as a
   tab instead (which downloads it if it can). */

/* WKNavigationActionPolicyDownload and WKNavigationResponsePolicyDownload
   (macOS 11.3), only ever passed once eclipse_little_download_tab found a tab
   to take the download (which needs WKDownload). */
static const WKNavigationActionPolicy kEclipseLittleActionPolicyDownload = (WKNavigationActionPolicy)2;
static const WKNavigationResponsePolicy kEclipseLittleResponsePolicyDownload = (WKNavigationResponsePolicy)2;

static SEL eclipse_little_download_selector(BOOL fromAction) {
    return fromAction ? @selector(webView:navigationAction:didBecomeDownload:)
                      : @selector(webView:navigationResponse:didBecomeDownload:);
}

/* A tab webview whose navigation delegate can take a download over (with
   `sel`): `preferred` (a popup's opener tab) if it is one, else a tab in a
   window, else any tab. nil without WKDownload or a tab. */
static WKWebView *eclipse_little_download_tab(WKWebView *preferred, SEL sel) {
    if (NSClassFromString(@"WKDownload") == Nil || gTabIdsByWebView == nil) return nil;
    BOOL (^usable)(WKWebView *) = ^BOOL(WKWebView *view) {
        id delegate = view.navigationDelegate;
        return delegate != nil && [delegate respondsToSelector:sel];
    };
    if (preferred != nil && [gTabIdsByWebView objectForKey:preferred] != nil && usable(preferred)) return preferred;
    WKWebView *fallback = nil;
    for (WKWebView *view in gTabIdsByWebView.keyEnumerator) {
        if (!usable(view)) continue;
        if (view.window != nil) return view;
        if (fallback == nil) fallback = view;
    }
    return fallback;
}

API_AVAILABLE(macos(11.3))
@interface EclipseLittleDownload : NSObject <WKDownloadDelegate>
/* wry's download delegate, which the tab's navigation delegate gave the
   download. */
@property(nonatomic, strong) id<WKDownloadDelegate> inner;
@property(nonatomic, strong) WKDownload *download;
@property(nonatomic, weak) EclipseLittleWindow *window;
@end

/* Downloads from Little Eclipse windows still running (a WKDownload holds
   its delegate weakly). */
static NSMutableSet *gLittleDownloads = nil;

@implementation EclipseLittleDownload

- (void)download:(WKDownload *)download
    decideDestinationUsingResponse:(NSURLResponse *)response
                 suggestedFilename:(NSString *)suggestedFilename
                 completionHandler:(void (^)(NSURL *))completionHandler {
    __weak EclipseLittleWindow *window = self.window;
    [self.inner download:download
        decideDestinationUsingResponse:response
                     suggestedFilename:suggestedFilename
                     completionHandler:^(NSURL *destination) {
                         completionHandler(destination);
                         // nil: tab.rs refused it (a page, not a file); the
                         // window stays as it is, like a tab.
                         if (destination == nil) return;
                         dispatch_async(dispatch_get_main_queue(), ^{
                             [window downloadStarted];
                         });
                     }];
}

- (void)downloadDidFinish:(WKDownload *)download {
    if ([self.inner respondsToSelector:@selector(downloadDidFinish:)]) [self.inner downloadDidFinish:download];
    [self finished];
}

- (void)download:(WKDownload *)download didFailWithError:(NSError *)error resumeData:(NSData *)resumeData {
    if ([self.inner respondsToSelector:@selector(download:didFailWithError:resumeData:)]) {
        [self.inner download:download didFailWithError:error resumeData:resumeData];
    }
    [self finished];
}

/* Released on the next turn of the run loop: WebKit is still calling it. */
- (void)finished {
    EclipseLittleDownload *me = self;
    dispatch_async(dispatch_get_main_queue(), ^{
        me.download = nil;
        [gLittleDownloads removeObject:me];
    });
}

/* Anything else WKDownloadDelegate asks (redirects, authentication) is for
   wry's delegate. */
- (BOOL)respondsToSelector:(SEL)selector {
    return [super respondsToSelector:selector] || [self.inner respondsToSelector:selector];
}

- (id)forwardingTargetForSelector:(SEL)selector {
    return [self.inner respondsToSelector:selector] ? self.inner : nil;
}

@end

/* Hands `download` (from `navigation`, the WKNavigationAction or
   WKNavigationResponse that became it) over to a tab's navigation delegate,
   with an EclipseLittleDownload in between. Cancels it if no tab takes it. */
API_AVAILABLE(macos(11.3))
static void eclipse_little_take_download(EclipseLittleWindow *lw, WKWebView *preferredTab, WKDownload *download,
                                         id navigation, BOOL fromAction) {
    SEL sel = eclipse_little_download_selector(fromAction);
    WKWebView *tab = eclipse_little_download_tab(preferredTab, sel);
    id delegate = tab.navigationDelegate;
    if (delegate != nil) {
        ((void (*)(id, SEL, id, id, id))objc_msgSend)(delegate, sel, tab, navigation, download);
    }
    id<WKDownloadDelegate> inner = download.delegate;
    if (inner == nil) {
        if (eclipse_perf_enabled()) fprintf(stderr, "[little] no tab took a download, cancelled\n");
        [download cancel:nil];
        return;
    }
    EclipseLittleDownload *proxy = [[EclipseLittleDownload alloc] init];
    proxy.inner = inner;
    proxy.download = download;
    proxy.window = lw;
    if (gLittleDownloads == nil) gLittleDownloads = [NSMutableSet set];
    [gLittleDownloads addObject:proxy];
    download.delegate = proxy;
}

@implementation EclipseLittleWindow

- (void)refreshTitle {
    if (self.closed) return;
    EclipseLittleAddress *address = eclipse_little_address(self.displayURL);
    NSString *title = eclipse_little_clean_title(self.webView.title);
    // The title bar is hidden: the title is what Mission Control, the Window
    // menu and the Dock list. The address on the bar is ours.
    self.window.title = title.length > 0 ? title : [address.prefix stringByAppendingString:address.host];
    if (![self.address isEqual:address]) {
        self.address = address;
        NSButton *info = self.infoButton;
        info.image = eclipse_little_security_image(address.security);
        info.contentTintColor = address.security == EclipseLittleSecurityNotSecure ? [NSColor systemOrangeColor]
                                                                                   : [NSColor secondaryLabelColor];
        info.accessibilityLabel = address.security == EclipseLittleSecurityNotSecure ? @"Not secure: site information"
                                                                                      : @"Site information";
        self.hostButton.accessibilityLabel = [@"Address: " stringByAppendingString:address.plain];
        [self.infoPopover close];
        [self fitHostLabel];
    }
}

/* The address button's title: the address, shortened to the button's width
   by eclipse_little_fit_address rather than by the cell (which would cut
   "Not Secure" off the front, or the host before the path). Again whenever
   the button changes size. */
- (void)fitHostLabel {
    NSButton *host = self.hostButton;
    if (host == nil || self.closed) return;
    EclipseLittleAddress *address = self.address ?: eclipse_little_address(nil);
    NSAttributedString *shown = eclipse_little_fit_address(host, address);
    if (![host.attributedTitle isEqualToAttributedString:shown]) host.attributedTitle = shown;
}

- (void)applyTheme {
    if (self.closed) return;
    NSColor *frame = eclipse_little_frame_color();
    self.window.appearance = [NSAppearance appearanceNamed:gLittleFrameDark ? NSAppearanceNameDarkAqua
                                                                             : NSAppearanceNameAqua];
    self.window.backgroundColor = frame;
    self.container.layer.backgroundColor = frame.CGColor;
    NSColor *border = gLittleFrameDark ? [NSColor colorWithCalibratedWhite:1.0 alpha:0.13]
                                       : [NSColor colorWithCalibratedWhite:0.0 alpha:0.12];
    self.webView.layer.borderColor = border.CGColor;
    NSColor *field = gLittleFrameDark ? [NSColor colorWithCalibratedWhite:1.0 alpha:0.10]
                                      : [NSColor colorWithCalibratedWhite:0.0 alpha:0.07];
    self.addressBox.layer.backgroundColor = field.CGColor;
    self.copiedLabel.layer.backgroundColor = field.CGColor;
    NSColor *line = gLittleFrameDark ? [NSColor colorWithCalibratedWhite:1.0 alpha:0.16]
                                     : [NSColor colorWithCalibratedWhite:0.0 alpha:0.14];
    self.separator.layer.backgroundColor = line.CGColor;
    self.texture.needsDisplay = YES;
}

- (void)layoutChrome {
    NSView *container = self.container;
    if (container == nil || self.closed) return;
    NSRect bounds = container.bounds;
    CGFloat mid = 0, leading = 0;
    eclipse_little_bar_metrics(self.window, &mid, &leading);
    CGFloat bar = MAX(kEclipseLittleBarHeight, ceil(mid * 2.0));
    CGFloat inset = kEclipseLittleInset;
    self.texture.frame = NSMakeRect(0, 0, NSWidth(bounds), bar);
    self.webView.frame = NSMakeRect(inset, bar, MAX(0.0, NSWidth(bounds) - 2.0 * inset),
                                    MAX(0.0, NSHeight(bounds) - bar - inset));
    // Right to left: Open in Eclipse | Copy Link.
    CGFloat button = 24.0, top = floor(mid - button / 2.0);
    self.openButton.frame = NSMakeRect(NSWidth(bounds) - 8.0 - button, top, button, button);
    self.separator.frame = NSMakeRect(NSMinX(self.openButton.frame) - 5.0, floor(mid - 7.0), 1.0, 14.0);
    self.linkButton.frame = NSMakeRect(NSMinX(self.separator.frame) - 4.0 - button, top, button, button);
    CGFloat right = NSMinX(self.linkButton.frame) - 4.0;
    NSSize copied = self.copiedLabel.intrinsicContentSize;
    CGFloat copiedWidth = isfinite(copied.width) ? ceil(copied.width) + 14.0 : 60.0;
    self.copiedLabel.frame = NSMakeRect(right - copiedWidth, floor(mid - 9.0), copiedWidth, 18.0);
    // Left to right: site information, then the address.
    self.infoButton.frame = NSMakeRect(leading - 4.0, top, button, button);
    CGFloat start = NSMaxX(self.infoButton.frame) + 1.0;
    self.hostButton.frame = NSMakeRect(start, floor(mid - 12.0), MAX(40.0, right - start), 24.0);
    [self fitHostLabel];
    NSRect box = NSMakeRect(leading - 4.0, floor(mid - 12.0), MAX(40.0, right - leading + 4.0), 24.0);
    self.addressBox.frame = box;
    CGFloat textHeight = ceil(self.addressField.intrinsicContentSize.height);
    if (!isfinite(textHeight) || textHeight <= 0 || textHeight > NSHeight(box)) textHeight = 17.0;
    self.addressField.frame = NSMakeRect(7.0, floor((NSHeight(box) - textHeight) / 2.0), NSWidth(box) - 14.0, textHeight);
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

/* -- the top bar */

/* Shows the full address of the committed document (selected) for editing. */
- (void)beginEditingAddress {
    if (self.closed) return;
    if (self.window.attachedSheet != nil) {
        NSBeep();
        return;
    }
    NSURL *url = self.displayURL;
    self.addressField.stringValue = eclipse_is_web_scheme(url.scheme) ? (url.absoluteString ?: @"") : @"";
    self.editing = YES;
    [self.infoPopover close];
    self.hostButton.hidden = YES;
    self.infoButton.hidden = YES;
    self.addressBox.hidden = NO;
    self.addressField.hidden = NO;
    [self.window makeFirstResponder:self.addressField];
    [self.addressField selectText:nil];
}

/* Back to the host label. `focusPage` hands the keyboard to the page. */
- (void)endEditingAddress:(BOOL)focusPage {
    if (!self.editing) return;
    // Cleared first: hiding the field ends its editing session, which must
    // not come back here.
    self.editing = NO;
    if (focusPage && !self.closed) [self.window makeFirstResponder:self.webView];
    self.addressField.hidden = YES;
    self.addressBox.hidden = YES;
    self.hostButton.hidden = NO;
    self.infoButton.hidden = NO;
}

/* Return in the address field: Rust resolves the text (an address or a
   search, see app::little_window_url_from_input) and calls
   eclipse_little_window_load. The text is what the user typed. */
- (void)submitAddress {
    NSString *text = [self.addressField.stringValue
        stringByTrimmingCharactersInSet:[NSCharacterSet whitespaceAndNewlineCharacterSet]];
    [self endEditingAddress:YES];
    if (text.length == 0 || self.closed) return;
    if (text.length > kEclipseLittleMaxAddressLength || gCallback == NULL) {
        NSBeep();
        return;
    }
    NSString *command = [NSString stringWithFormat:@"little_window_address:%llu:%@",
                                                   (unsigned long long)self.serial, text];
    gCallback(command.UTF8String);
}

- (void)hostClicked:(id)sender {
    (void)sender;
    [self beginEditingAddress];
}

- (void)openClicked:(id)sender {
    (void)sender;
    [self.infoPopover close];
    [self openInEclipse];
}

- (void)copyClicked:(id)sender {
    (void)sender;
    [self.infoPopover close];
    [self copyLink];
}

- (void)infoClicked:(id)sender {
    (void)sender;
    [self showSiteInfo];
}

/* Copies the committed page's address (http(s) only) and says so for a
   moment: "Copied" on the bar and a checkmark on the button. */
- (void)copyLink {
    if (self.closed) return;
    NSURL *url = self.displayURL;
    NSString *string = url.absoluteString;
    if (!eclipse_is_web_scheme(url.scheme) || string.length == 0) {
        NSBeep();
        return;
    }
    NSPasteboard *pasteboard = eclipse_pasteboard();
    [pasteboard clearContents];
    NSPasteboardItem *item = [[NSPasteboardItem alloc] init];
    [item setString:string forType:NSPasteboardTypeString];
    [item setString:string forType:NSPasteboardTypeURL];
    [pasteboard writeObjects:@[ item ]];
    eclipse_little_log(@"copied link", self);

    NSUInteger generation = ++self.copiedGeneration;
    self.copiedLabel.hidden = NO;
    self.linkButton.image = eclipse_little_symbol(@[ @"checkmark" ], @"Copied") ?: self.linkButton.image;
    __weak EclipseLittleWindow *weakSelf = self;
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(1.5 * NSEC_PER_SEC)), dispatch_get_main_queue(), ^{
        EclipseLittleWindow *me = weakSelf;
        if (me == nil || me.closed || me.copiedGeneration != generation) return;
        me.copiedLabel.hidden = YES;
        me.linkButton.image = eclipse_little_symbol(@[ @"link" ], @"Copy Link") ?: me.linkButton.image;
    });
}

/* The site-information popover under the "i" button: whether the connection
   is secure, the full host (from the committed URL, like the bar), Copy Link
   and Open in Eclipse. */
- (void)showSiteInfo {
    if (self.closed) return;
    if (self.infoPopover.shown) {
        [self.infoPopover close];
        return;
    }
    EclipseLittleAddress *address = self.address ?: eclipse_little_address(nil);
    BOOL web = eclipse_is_web_scheme(self.displayURL.scheme);
    NSString *heading = @"No page";
    NSString *detail = @"This window is not showing a web page.";
    NSColor *tint = [NSColor secondaryLabelColor];
    NSArray<NSString *> *symbol = @[ @"globe" ];
    if (address.security == EclipseLittleSecuritySecure) {
        heading = @"Connection is secure";
        detail = @"Information you send to this site is private.";
        symbol = @[ @"lock.fill" ];
    } else if (address.security == EclipseLittleSecurityNotSecure) {
        heading = @"Connection is not secure";
        detail = @"Don't enter passwords or payment details here: others on the network could see them.";
        tint = [NSColor systemOrangeColor];
        symbol = @[ @"exclamationmark.triangle.fill" ];
    }

    NSImageView *icon = [[NSImageView alloc] initWithFrame:NSZeroRect];
    icon.image = eclipse_little_symbol(symbol, heading);
    icon.contentTintColor = tint;
    NSTextField *headingLabel = [NSTextField labelWithString:heading];
    headingLabel.font = [NSFont systemFontOfSize:13 weight:NSFontWeightSemibold];
    headingLabel.identifier = @"eclipse-little-info-heading";
    NSStackView *top = [NSStackView stackViewWithViews:@[ icon, headingLabel ]];
    top.spacing = 6;

    NSTextField *hostLabel = [NSTextField labelWithString:address.host];
    hostLabel.font = eclipse_little_address_font(NSFontWeightSemibold);
    hostLabel.selectable = YES;
    hostLabel.lineBreakMode = NSLineBreakByTruncatingHead;
    hostLabel.identifier = @"eclipse-little-info-host";
    NSTextField *detailLabel = [NSTextField wrappingLabelWithString:detail];
    detailLabel.font = [NSFont systemFontOfSize:11.5];
    detailLabel.textColor = [NSColor secondaryLabelColor];
    detailLabel.preferredMaxLayoutWidth = 260;

    NSButton *copy = [NSButton buttonWithTitle:@"Copy Link" target:self action:@selector(copyClicked:)];
    copy.identifier = @"eclipse-little-info-copy";
    copy.enabled = web;
    NSButton *open = [NSButton buttonWithTitle:@"Open in Eclipse" target:self action:@selector(openClicked:)];
    open.identifier = @"eclipse-little-info-open";
    open.enabled = web;
    NSStackView *buttons = [NSStackView stackViewWithViews:@[ copy, open ]];
    buttons.spacing = 8;

    NSStackView *stack = [NSStackView stackViewWithViews:@[ top, hostLabel, detailLabel, buttons ]];
    stack.orientation = NSUserInterfaceLayoutOrientationVertical;
    stack.alignment = NSLayoutAttributeLeading;
    stack.spacing = 8;
    stack.edgeInsets = NSEdgeInsetsMake(14, 14, 14, 14);
    [stack setCustomSpacing:4 afterView:top];
    // A fixed text width: the popover takes the stack's fitting size.
    [hostLabel.widthAnchor constraintLessThanOrEqualToConstant:260].active = YES;
    [detailLabel.widthAnchor constraintEqualToConstant:260].active = YES;
    NSSize size = stack.fittingSize;
    stack.frame = NSMakeRect(0, 0, size.width, size.height);

    NSViewController *controller = [[NSViewController alloc] init];
    controller.view = stack;
    controller.preferredContentSize = size;
    NSPopover *popover = [[NSPopover alloc] init];
    popover.behavior = NSPopoverBehaviorTransient;
    popover.contentViewController = controller;
    popover.appearance = self.window.appearance;
    self.infoPopover = popover;
    // Below the button (the edge is in the button's own coordinates).
    [popover showRelativeToRect:self.infoButton.bounds
                         ofView:self.infoButton
                  preferredEdge:self.infoButton.isFlipped ? NSRectEdgeMaxY : NSRectEdgeMinY];
}

/* Moves the committed page into a new foreground tab of the browser window
   and closes this window. A popup's opener link is lost, as in Arc. */
- (void)openInEclipse {
    if (self.closed) return;
    NSURL *url = self.displayURL;
    if (!eclipse_is_web_scheme(url.scheme) || url.absoluteString.length == 0 || gCallback == NULL) {
        NSBeep();
        return;
    }
    eclipse_little_log(@"open in Eclipse", self);
    NSString *command = [@"little_window_new_tab:" stringByAppendingString:url.absoluteString];
    gCallback(command.UTF8String);
    NSWindow *browser = gBrowserWindow;
    if (browser != nil) [browser makeKeyAndOrderFront:nil];
    eclipse_end_all_sheets(self.window);
    [self.window close];
}

/* -- downloads (see above) */

/* The tab a download from this window preferably goes through: a popup's
   opener tab (nil for a standalone window and its popups: any tab). */
- (WKWebView *)downloadTab {
    return self.standalone ? nil : self.opener;
}

/* No page has been on screen: nothing committed, and the opener has not
   written into the initial blank document either. */
- (BOOL)showsNoPage {
    if (self.webView.backForwardList.currentItem != nil) return NO;
    NSURL *shown = self.displayURL;
    return shown == nil || [shown.scheme.lowercaseString isEqualToString:@"about"];
}

/* A file from this window went elsewhere (a download, or a tab). A window
   opened for it, which has shown no page, closes. */
- (void)fileLeft {
    if (self.closed || ![self showsNoPage]) return;
    if (eclipse_perf_enabled()) {
        fprintf(stderr, "[little] closing a window that only held a file standalone=%d\n", self.standalone ? 1 : 0);
    }
    eclipse_end_all_sheets(self.window);
    [self.window close];
}

- (void)downloadStarted {
    if (self.closed) return;
    eclipse_little_log(@"download started", self);
    [self fileLeft];
}

- (void)webView:(WKWebView *)webView
    navigationResponse:(WKNavigationResponse *)navigationResponse
     didBecomeDownload:(WKDownload *)download API_AVAILABLE(macos(11.3)) {
    (void)webView;
    eclipse_little_take_download(self, [self downloadTab], download, navigationResponse, NO);
}

- (void)webView:(WKWebView *)webView
    navigationAction:(WKNavigationAction *)navigationAction
    didBecomeDownload:(WKDownload *)download API_AVAILABLE(macos(11.3)) {
    (void)webView;
    eclipse_little_take_download(self, [self downloadTab], download, navigationAction, YES);
}

/* NSTextFieldDelegate */

- (BOOL)control:(NSControl *)control textView:(NSTextView *)textView doCommandBySelector:(SEL)selector {
    (void)textView;
    if (control != self.addressField) return NO;
    if (selector == @selector(insertNewline:)) {
        [self submitAddress];
        return YES;
    }
    if (selector == @selector(cancelOperation:)) {
        [self endEditingAddress:YES];
        return YES;
    }
    return NO;
}

/* Focus went elsewhere (a click into the page, another window). Checked on
   the next turn of the run loop: AppKit also ends an editing session only to
   start the next one at once (selecting all the text does). */
- (void)controlTextDidEndEditing:(NSNotification *)notification {
    if (notification.object != self.addressField || !self.editing) return;
    __weak EclipseLittleWindow *weakSelf = self;
    dispatch_async(dispatch_get_main_queue(), ^{
        EclipseLittleWindow *me = weakSelf;
        if (me == nil || !me.editing || me.closed) return;
        NSText *editor = me.addressField.currentEditor;
        if (editor != nil && me.window.firstResponder == editor) return;
        [me endEditingAddress:NO];
    });
}

/* NSWindowDelegate */

- (void)windowDidResize:(NSNotification *)notification {
    (void)notification;
    [self layoutChrome];
}

- (void)windowDidEndLiveResize:(NSNotification *)notification {
    (void)notification;
    // The size the user gave a standalone window is the next one's.
    if (self.standalone && !self.closed && (self.window.styleMask & NSWindowStyleMaskFullScreen) == 0) {
        NSSize size = self.window.contentView.bounds.size;
        gLittleStandaloneSize = NSMakeSize(MAX(kEclipseLittleMinWidth, size.width - kEclipseLittleChromeWidth),
                                           MAX(kEclipseLittleMinHeight, size.height - kEclipseLittleChromeHeight));
    }
}

- (void)windowWillClose:(NSNotification *)notification {
    (void)notification;
    if (self.closed) return;
    // A dialog still showing is answered (Cancel) before the page goes away.
    eclipse_end_all_sheets(self.window);
    self.editing = NO;
    self.closed = YES;
    [self stopObserving];
    self.addressField.delegate = nil;
    [self.infoPopover close];
    self.infoPopover = nil;
    self.infoButton.target = nil;
    self.hostButton.target = nil;
    self.linkButton.target = nil;
    self.openButton.target = nil;
    WKWebView *webView = self.webView;
    webView.UIDelegate = nil;
    webView.navigationDelegate = nil;
    [webView stopLoading];
    // Released on the next turn of the run loop: the window is still in the
    // middle of closing. The webview leaves its window, so popups this window
    // is the root opener of (a standalone window's) close too.
    EclipseLittleWindow *me = self;
    dispatch_async(dispatch_get_main_queue(), ^{
        [webView removeFromSuperview];
        [gLittleWindows removeObject:me];
        if (eclipse_perf_enabled()) {
            fprintf(stderr, "[little] closed standalone=%d open=%lu\n", me.standalone ? 1 : 0,
                    (unsigned long)gLittleWindows.count);
        }
        if (gLittleWindows.count == 0) {
            [gLittleOpenerTimer invalidate];
            gLittleOpenerTimer = nil;
        } else {
            eclipse_little_check_openers();
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
    // A link with `download` (to a web page, or a blob/data file the page
    // made) downloads, as in a tab (WKNavigationAction.shouldPerformDownload,
    // macOS 11.3).
    BOOL wantsDownload = [action respondsToSelector:@selector(shouldPerformDownload)] &&
                         ((BOOL (*)(id, SEL))objc_msgSend)(action, @selector(shouldPerformDownload));
    if (wantsDownload &&
        (eclipse_is_web_scheme(scheme) || [scheme isEqualToString:@"blob"] || [scheme isEqualToString:@"data"]) &&
        eclipse_little_download_tab([self downloadTab], eclipse_little_download_selector(YES)) != nil) {
        decisionHandler(kEclipseLittleActionPolicyDownload);
        return;
    }
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
    if (response.canShowMIMEType) {
        decisionHandler(WKNavigationResponsePolicyAllow);
        return;
    }
    // A file: downloaded as a tab would (see "downloads" above).
    if (eclipse_little_download_tab([self downloadTab], eclipse_little_download_selector(NO)) != nil) {
        decisionHandler(kEclipseLittleResponsePolicyDownload);
        return;
    }
    decisionHandler(WKNavigationResponsePolicyCancel);
    // No download here: a file that would have been the window's page opens
    // as a tab instead, rather than vanish.
    NSURL *url = response.response.URL;
    if (self.closed || !response.isForMainFrame || !eclipse_is_web_scheme(url.scheme) ||
        url.absoluteString.length == 0 || gCallback == NULL) {
        return;
    }
    if (eclipse_perf_enabled()) fprintf(stderr, "[little] cannot download here, opening the file as a tab\n");
    NSString *command = [@"little_window_new_tab:" stringByAppendingString:url.absoluteString];
    gCallback(command.UTF8String);
    __weak EclipseLittleWindow *weakSelf = self;
    dispatch_async(dispatch_get_main_queue(), ^{
        [weakSelf fileLeft];
    });
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
    // The chain's root: the tab, or this window if it is standalone.
    WKWebView *root = self.standalone ? self.webView : self.opener;
    // A window.open with a user gesture that keeps its opener (see
    // eclipse_request_wants_popup) nests another Little Eclipse window, as it
    // would from a tab (within the same limits).
    if (eclipse_request_wants_popup(action, features) && root != nil &&
        eclipse_action_user_initiated(action) && eclipse_little_initial_url_allowed(url)) {
        BOOL hasPosition = features.x != nil && features.y != nil;
        NSPoint topLeft = hasPosition ? NSMakePoint(features.x.doubleValue, features.y.doubleValue) : NSZeroPoint;
        EclipseLittleWindow *nested = eclipse_little_window_create(
            configuration, root, NSMakeSize(features.width.doubleValue, features.height.doubleValue),
            hasPosition, topLeft, NO);
        if (nested != nil) return nested.webView;
    }
    // Anything else (a target=_blank link, a noopener window.open, one
    // without a gesture, a popup over the limit): a web page opens as a tab,
    // as it would from a tab.
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

/* window.print() in the window. */
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

/* Dialogs are sheets on the window. The completion handler runs exactly
   once: immediately with Cancel when the window is not on screen or already
   shows a dialog, otherwise when the sheet ends (also when the window
   closes). */
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
    [self endEditingAddress:NO];
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
    [self endEditingAddress:NO];
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
    [self endEditingAddress:NO];
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
    [self endEditingAddress:NO];
    NSOpenPanel *panel = [NSOpenPanel openPanel];
    panel.canChooseFiles = YES;
    panel.allowsMultipleSelection = parameters.allowsMultipleSelection;
    panel.canChooseDirectories = parameters.allowsDirectories;
    [panel beginSheetModalForWindow:self.window completionHandler:^(NSModalResponse response) {
        completionHandler(response == NSModalResponseOK ? panel.URLs : nil);
    }];
}

@end

/* Whether `opener` (the root of a popup chain) is still showing. */
static BOOL eclipse_little_opener_alive(WKWebView *opener) {
    return opener != nil && opener.superview != nil && opener.window != nil;
}

static void eclipse_little_check_openers(void) {
    for (EclipseLittleWindow *lw in [gLittleWindows allObjects]) {
        // Standalone windows have no opener and never close for one.
        if (!lw.closed && !lw.standalone && !eclipse_little_opener_alive(lw.opener)) {
            if (eclipse_perf_enabled()) {
                fprintf(stderr, "[little] opener is gone, closing\n");
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

/* Moves `frame` inside `visible` (it already fits: it is at most 90% of
   it). */
static NSRect eclipse_clamp_frame(NSRect frame, NSRect visible) {
    if (NSMaxX(frame) > NSMaxX(visible)) frame.origin.x = NSMaxX(visible) - frame.size.width;
    if (NSMinX(frame) < NSMinX(visible)) frame.origin.x = NSMinX(visible);
    if (NSMaxY(frame) > NSMaxY(visible)) frame.origin.y = NSMaxY(visible) - frame.size.height;
    if (NSMinY(frame) < NSMinY(visible)) frame.origin.y = NSMinY(visible);
    return frame;
}

/* The largest window (page area plus chrome) a Little Eclipse window may
   have on a screen whose visible frame is `visible`. */
static NSSize eclipse_little_max_window_size(NSRect visible) {
    return NSMakeSize(MAX(kEclipseLittleMinWidth + kEclipseLittleChromeWidth, floor(visible.size.width * 0.9)),
                      MAX(kEclipseLittleMinHeight + kEclipseLittleChromeHeight, floor(visible.size.height * 0.9)));
}

/* `frame` (a window frame a page asked for) within the window limits, its
   top-left corner where it was asked for, and moved inside `visible`. */
static NSRect eclipse_little_fit_frame(NSWindow *window, NSRect frame, NSRect visible) {
    NSRect content = [window contentRectForFrameRect:frame];
    NSSize max = eclipse_little_max_window_size(visible);
    content.size.width = MIN(MAX(content.size.width, kEclipseLittleMinWidth + kEclipseLittleChromeWidth), max.width);
    content.size.height =
        MIN(MAX(content.size.height, kEclipseLittleMinHeight + kEclipseLittleChromeHeight), max.height);
    NSRect fitted = [window frameRectForContentRect:content];
    fitted.origin.x = frame.origin.x;
    fitted.origin.y = NSMaxY(frame) - fitted.size.height;
    return eclipse_clamp_frame(fitted, visible);
}

/* Whether one more window may open: at most kEclipseLittleMaxTotal in all,
   and kEclipseLittleMaxPerTab for the root opener `opener` (nil for a
   standalone window, which only counts toward the total). */
static BOOL eclipse_little_has_room(WKWebView *opener) {
    NSUInteger total = 0, forOpener = 0;
    for (EclipseLittleWindow *lw in gLittleWindows) {
        if (lw.closed) continue;
        total++;
        if (opener != nil && lw.opener == opener) forOpener++;
    }
    return total < kEclipseLittleMaxTotal && (opener == nil || forOpener < kEclipseLittleMaxPerTab);
}

/* The screen the mouse pointer is on. */
static NSScreen *eclipse_screen_with_mouse(void) {
    NSPoint mouse = [NSEvent mouseLocation];
    for (NSScreen *screen in [NSScreen screens]) {
        if (NSMouseInRect(mouse, screen.frame, NO)) return screen;
    }
    return [NSScreen mainScreen];
}

static NSButton *eclipse_little_bar_button(NSString *identifier, id target, SEL action) {
    NSButton *button = [[NSButton alloc] initWithFrame:NSZeroRect];
    button.bezelStyle = NSBezelStyleTexturedRounded;
    button.showsBorderOnlyWhileMouseInside = YES;
    button.refusesFirstResponder = YES;
    button.target = target;
    button.action = action;
    button.identifier = identifier;
    return button;
}

static EclipseLittleWindow *eclipse_little_window_create(WKWebViewConfiguration *configuration,
                                                         WKWebView *opener,
                                                         NSSize requested,
                                                         BOOL hasPosition,
                                                         NSPoint topLeft,
                                                         BOOL standalone) {
    if (configuration == nil) return nil;
    if (standalone) {
        opener = nil;
    } else {
        if (!eclipse_little_opener_alive(opener)) return nil;
        // A popup of a standalone window is built on that window's fresh
        // configuration (no eclipse:// handler, no IPC); a tab's popup needs
        // the guards.
        BOOL fromStandalone = gLittleWebViews != nil && [gLittleWebViews containsObject:opener];
        if (!fromStandalone && (!gLittleGuardsReady || !eclipse_scheme_handler_guarded(configuration, @"eclipse"))) {
            if (eclipse_perf_enabled()) fprintf(stderr, "[little] guards missing, opening as a tab\n");
            return nil;
        }
    }
    if (!eclipse_little_has_room(opener)) {
        if (eclipse_perf_enabled()) fprintf(stderr, "[little] limit reached, opening as a tab\n");
        return nil;
    }

    NSWindow *anchor = standalone ? nil : (opener.window ?: eclipse_browser_window());
    NSScreen *screen = standalone ? eclipse_screen_with_mouse() : (anchor.screen ?: [NSScreen mainScreen]);
    if (screen == nil) return nil;
    NSRect visible = screen.visibleFrame;

    // The page area: what was asked for (a missing dimension, 0, gets the
    // default), or for a standalone window the last size the user gave one.
    NSSize maxWindow = eclipse_little_max_window_size(visible);
    if (standalone) {
        requested = gLittleStandaloneSize;
    }
    CGFloat width = isfinite(requested.width) && requested.width > 0 ? requested.width
                                                                      : kEclipseLittleDefaultWidth;
    CGFloat height = isfinite(requested.height) && requested.height > 0 ? requested.height
                                                                         : kEclipseLittleDefaultHeight;
    width = MIN(MAX(width, kEclipseLittleMinWidth), maxWindow.width - kEclipseLittleChromeWidth);
    height = MIN(MAX(height, kEclipseLittleMinHeight), maxWindow.height - kEclipseLittleChromeHeight);
    NSRect contentRect =
        NSMakeRect(0, 0, width + kEclipseLittleChromeWidth, height + kEclipseLittleChromeHeight);

    NSWindowStyleMask style = NSWindowStyleMaskTitled | NSWindowStyleMaskClosable |
                              NSWindowStyleMaskMiniaturizable | NSWindowStyleMaskResizable |
                              NSWindowStyleMaskFullSizeContentView;
    NSWindow *window = [[NSWindow alloc] initWithContentRect:contentRect
                                                   styleMask:style
                                                     backing:NSBackingStoreBuffered
                                                       defer:NO];
    window.releasedWhenClosed = NO;
    window.restorable = NO;
    // A compact frame: no visible title bar, the traffic lights on our bar.
    window.titlebarAppearsTransparent = YES;
    window.titleVisibility = NSWindowTitleHidden;
    // Shown over the browser window also when that is in full screen.
    window.collectionBehavior |= NSWindowCollectionBehaviorFullScreenAuxiliary;
    window.contentMinSize = NSMakeSize(kEclipseLittleMinWidth + kEclipseLittleChromeWidth,
                                       kEclipseLittleMinHeight + kEclipseLittleChromeHeight);
    if ([window respondsToSelector:@selector(setTabbingMode:)]) {
        window.tabbingMode = NSWindowTabbingModeDisallowed;
    }

    WKWebView *webView = nil;
    @try {
        webView = [[WKWebView alloc] initWithFrame:NSMakeRect(0, 0, width, height) configuration:configuration];
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
    if (opener != nil) webView.customUserAgent = opener.customUserAgent;
    webView.allowsMagnification = YES;
    webView.allowsBackForwardNavigationGestures = YES;
    // respondsToSelector rather than @available: the 13.3 check is above the
    // 11.0 deployment target and would need compiler-rt, which rustc's link
    // line does not include.
    if ([webView respondsToSelector:@selector(setInspectable:)]) {
        [webView setValue:@YES forKey:@"inspectable"];
    }
    // The page as a card, like a tab in the browser window.
    webView.wantsLayer = YES;
    webView.layer.cornerRadius = kEclipseLittleCardRadius;
    if (@available(macOS 10.15, *)) {
        webView.layer.cornerCurve = kCACornerCurveContinuous;
    }
    webView.layer.masksToBounds = YES;
    webView.layer.borderWidth = 1.0;

    EclipseLittleWindow *lw = [[EclipseLittleWindow alloc] init];

    EclipseLittleContentView *container = [[EclipseLittleContentView alloc] initWithFrame:contentRect];
    container.wantsLayer = YES;
    EclipseLittleBarTexture *texture = [[EclipseLittleBarTexture alloc] initWithFrame:NSZeroRect];
    [container addSubview:texture];
    [container addSubview:webView];

    NSButton *info = eclipse_little_bar_button(@"eclipse-little-info", lw, @selector(infoClicked:));
    info.imagePosition = NSImageOnly;
    info.image = eclipse_little_security_image(EclipseLittleSecurityNone);
    if (info.image == nil) info.title = @"i";
    info.contentTintColor = [NSColor secondaryLabelColor];
    info.toolTip = @"Site information";
    [container addSubview:info];

    NSButton *host = eclipse_little_bar_button(@"eclipse-little-host", lw, @selector(hostClicked:));
    host.alignment = NSTextAlignmentLeft;
    host.imagePosition = NSNoImage;
    host.font = eclipse_little_address_font(NSFontWeightRegular);
    // -fitHostLabel shortens the address to fit; the title's own truncation
    // is a last resort.
    host.cell.lineBreakMode = NSLineBreakByTruncatingHead;
    host.toolTip = @"Edit the address (⌘L)";
    [container addSubview:host];

    NSTextField *field = [[NSTextField alloc] initWithFrame:NSZeroRect];
    field.identifier = @"eclipse-little-address";
    field.bezeled = NO;
    field.bordered = NO;
    field.drawsBackground = NO;
    field.focusRingType = NSFocusRingTypeNone;
    field.font = eclipse_little_address_font(NSFontWeightRegular);
    field.usesSingleLineMode = YES;
    field.cell.scrollable = YES;
    field.cell.wraps = NO;
    field.placeholderString = @"Search or Enter URL…";
    field.hidden = YES;
    field.delegate = lw;
    NSView *box = [[NSView alloc] initWithFrame:NSZeroRect];
    box.wantsLayer = YES;
    box.layer.cornerRadius = 6.0;
    box.hidden = YES;
    [box addSubview:field];
    [container addSubview:box];

    NSTextField *copied = [NSTextField labelWithString:@"Copied"];
    copied.identifier = @"eclipse-little-copied";
    copied.font = [NSFont systemFontOfSize:11 weight:NSFontWeightSemibold];
    copied.textColor = [NSColor labelColor];
    copied.alignment = NSTextAlignmentCenter;
    copied.wantsLayer = YES;
    copied.layer.cornerRadius = 9.0;
    copied.hidden = YES;
    [container addSubview:copied];

    NSButton *copy = eclipse_little_bar_button(@"eclipse-little-copy", lw, @selector(copyClicked:));
    copy.image = eclipse_little_symbol(@[ @"link" ], @"Copy Link");
    if (copy.image != nil) {
        copy.imagePosition = NSImageOnly;
    } else {
        copy.title = @"Copy";
    }
    copy.contentTintColor = [NSColor secondaryLabelColor];
    copy.toolTip = @"Copy Link";
    copy.accessibilityLabel = @"Copy Link";
    [container addSubview:copy];

    NSView *separator = [[NSView alloc] initWithFrame:NSZeroRect];
    separator.wantsLayer = YES;
    [container addSubview:separator];

    NSButton *open = eclipse_little_bar_button(@"eclipse-little-open", lw, @selector(openClicked:));
    open.image = eclipse_little_symbol(@[ @"arrow.up.forward.app", @"arrow.up.right.square" ], @"Open in Eclipse");
    if (open.image != nil) {
        open.imagePosition = NSImageOnly;
    } else {
        open.title = @"Open";
    }
    open.contentTintColor = [NSColor secondaryLabelColor];
    open.toolTip = @"Open in Eclipse (⌘O)";
    open.accessibilityLabel = @"Open in Eclipse";
    [container addSubview:open];

    window.contentView = container;

    // Where: a standalone window in the middle of the screen with the
    // pointer (a step down from the last one if that is already there). A
    // popup at the requested spot if the whole window fits on a screen
    // there, else a step down from the last popup still open, else centred
    // over the browser window.
    NSRect frame = [window frameRectForContentRect:contentRect];
    BOOL placed = NO;
    EclipseLittleWindow *previous = gLastLittleWindow;
    BOOL previousShown = previous != nil && !previous.closed && previous.window.isVisible;
    if (standalone) {
        frame.origin.x = floor(NSMidX(visible) - frame.size.width / 2.0);
        frame.origin.y = floor(NSMidY(visible) - frame.size.height / 2.0);
        if (previousShown && NSEqualPoints(previous.window.frame.origin, frame.origin)) {
            frame.origin.x += 24.0;
            frame.origin.y -= 24.0;
        }
        frame = eclipse_clamp_frame(frame, visible);
        placed = YES;
    }
    if (!placed && hasPosition && isfinite(topLeft.x) && isfinite(topLeft.y)) {
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
    if (!placed && previousShown) {
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

    lw.window = window;
    lw.webView = webView;
    lw.opener = opener;
    lw.standalone = standalone;
    lw.serial = gLittleNextSerial++;
    lw.container = container;
    lw.texture = texture;
    lw.infoButton = info;
    lw.hostButton = host;
    lw.addressField = field;
    lw.addressBox = box;
    lw.copiedLabel = copied;
    lw.linkButton = copy;
    lw.separator = separator;
    lw.openButton = open;
    container.owner = lw;
    webView.UIDelegate = lw;
    webView.navigationDelegate = lw;
    window.delegate = lw;
    [webView addObserver:lw forKeyPath:@"URL" options:0 context:kEclipseLittleKVOContext];
    [webView addObserver:lw forKeyPath:@"loading" options:0 context:kEclipseLittleKVOContext];
    [webView addObserver:lw forKeyPath:@"title" options:0 context:kEclipseLittleKVOContext];
    lw.observing = YES;
    [lw applyTheme];
    [lw refreshTitle];
    [lw layoutChrome];

    if (gLittleWindows == nil) gLittleWindows = [NSMutableSet set];
    [gLittleWindows addObject:lw];
    gLastLittleWindow = lw;
    eclipse_little_start_opener_timer();

    [window makeKeyAndOrderFront:nil];
    // The traffic lights have their final place once the window is shown.
    [lw layoutChrome];
    if (eclipse_perf_enabled()) {
        fprintf(stderr, "[little] opened %.0fx%.0f standalone=%d open=%lu\n", width, height, standalone ? 1 : 0,
                (unsigned long)gLittleWindows.count);
    }
    return lw;
}

/* Whether the window.open being decided (inside wry's new-window handler)
   opens as a Little Eclipse window: it asked for a popup window or keeps its
   opener; see eclipse_request_wants_popup. */
int eclipse_popup_request_wants_popup(void) {
    return (gPopupRequestContext & ECLIPSE_POPUP_WANTS_POPUP) != 0;
}

/* Opens a Little Eclipse window for a popup requested by `openerPtr` (a tab's
   WKWebView), built on `configurationPtr` (the WKWebViewConfiguration WebKit
   handed the opener's UI delegate). `width`/`height` are the page area asked
   for (0: the default); `x`/`y` are the requested top-left in screen
   coordinates with y downwards from the primary screen's top.
   Only called from wry's new-window handler, inside the wrapped UI delegate
   method: a request without a user gesture gets no window.
   Returns the popup's WKWebView retained (+1; the caller owns that
   reference and hands it to WebKit), or NULL when no window may or could be
   made (the caller then opens the URL as a tab). */
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
            configuration, opener, NSMakeSize(width, height), hasPosition != 0, NSMakePoint(x, y), NO);
        if (lw == nil) return NULL;
        return (__bridge_retained void *)lw.webView;
    }
}

/* Opens a standalone Little Eclipse window (no opener): a link from another
   app (`url`, http(s) only) or a blank one with the address field focused
   (`url` empty, `focusAddress`). Built on a fresh configuration that shares
   the tabs' website data store (wry's tabs use the default store), with the
   tabs' `userAgent` (empty: WebKit's) and, if `adblock`, the native content
   blocker; no tab scripts or IPC. 1 when it opened, 0 when it may not (the
   overall limit) or the URL is not a web address: Rust then opens a tab. */
int eclipse_open_standalone_little_window(const char *url, const char *userAgent, int adblock, int focusAddress) {
    @autoreleasepool {
        NSURL *target = nil;
        if (url != NULL && url[0] != '\0') {
            target = eclipse_little_loadable_url(url);
            if (target == nil) {
                if (eclipse_perf_enabled()) fprintf(stderr, "[little] refused a standalone window for a non-web URL\n");
                return 0;
            }
        }
        WKWebViewConfiguration *configuration = [[WKWebViewConfiguration alloc] init];
        configuration.websiteDataStore = [WKWebsiteDataStore defaultDataStore];
        EclipseLittleWindow *lw =
            eclipse_little_window_create(configuration, nil, NSZeroSize, NO, NSZeroPoint, YES);
        if (lw == nil) return 0;
        NSString *agent = userAgent != NULL ? [NSString stringWithUTF8String:userAgent] : nil;
        if (agent.length > 0) lw.webView.customUserAgent = agent;
        if (adblock) eclipse_adblock_want_webview(lw.webView);
        if (target != nil) {
            [lw.webView loadRequest:[NSURLRequest requestWithURL:target]];
        }
        [NSApp activateIgnoringOtherApps:YES];
        if (focusAddress || target == nil) {
            [lw beginEditingAddress];
        } else {
            [lw.window makeFirstResponder:lw.webView];
        }
        return 1;
    }
}

static EclipseLittleWindow *eclipse_little_window_with_serial(uint64_t serial) {
    for (EclipseLittleWindow *lw in gLittleWindows) {
        if (!lw.closed && lw.serial == serial) return lw;
    }
    return nil;
}

/* Loads `url` (Rust's resolution of what was typed in the address field of
   window `windowId`) into that window. Only http(s) addresses with a host
   load; anything else (NULL: Rust found nothing to load) beeps. 1 when the
   load started. */
int eclipse_little_window_load(uint64_t windowId, const char *url) {
    @autoreleasepool {
        EclipseLittleWindow *lw = eclipse_little_window_with_serial(windowId);
        if (lw == nil) return 0;
        NSURL *target = eclipse_little_loadable_url(url);
        if (target == nil) {
            if (eclipse_perf_enabled()) fprintf(stderr, "[little] refused an address that is not a web page\n");
            NSBeep();
            return 0;
        }
        [lw.webView loadRequest:[NSURLRequest requestWithURL:target]];
        [lw.window makeFirstResponder:lw.webView];
        return 1;
    }
}

/* The current Space's theme colour (sRGB, 0…1) and whether it is dark; every
   Little Eclipse window, open or later, is framed in it. */
void eclipse_set_little_window_theme(double red, double green, double blue, int dark) {
    @autoreleasepool {
        if (!isfinite(red) || !isfinite(green) || !isfinite(blue)) return;
        gLittleFrameColor = [NSColor colorWithSRGBRed:MIN(MAX(red, 0.0), 1.0)
                                                green:MIN(MAX(green, 0.0), 1.0)
                                                 blue:MIN(MAX(blue, 0.0), 1.0)
                                                alpha:1.0];
        gLittleFrameDark = dark != 0;
        for (EclipseLittleWindow *lw in gLittleWindows) [lw applyTheme];
    }
}

/* Closes every Little Eclipse window (the app is quitting). */
void eclipse_close_all_little_windows(void) {
    @autoreleasepool {
        NSArray<EclipseLittleWindow *> *open = [gLittleWindows allObjects];
        NSUInteger count = 0;
        for (EclipseLittleWindow *lw in open) {
            if (lw.closed) continue;
            count++;
            [lw.window close];
        }
        if (eclipse_perf_enabled() && count > 0) {
            fprintf(stderr, "[little] closed all on quit count=%lu\n", (unsigned long)count);
        }
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
   is). Page commands act on the window; commands about a tab's page (Home,
   Bookmark, Find, which needs the tab's in-page find bar) are refused rather
   than sent to the hidden tab behind it. ⌘L edits the window's address and
   ⌘O opens its page in the browser window. YES when the command was handled
   here; the rest go to the browser (⌘W closes the window, see
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
                                              @"zoom_in", @"zoom_out", @"zoom_reset", @"open_devtools",
                                              @"open_in_eclipse", @"focus_omnibar" ]];
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
    } else if ([action isEqualToString:@"open_in_eclipse"]) {
        [lw openInEclipse];
    } else if ([action isEqualToString:@"focus_omnibar"]) {
        [lw beginEditingAddress];
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
        // The window's own Web Inspector (WebKit's _inspector SPI).
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

/* ------------------------------------------------------------- New Little Eclipse Window bar */
/* ⌥⌘N, as in Arc: a floating "Search or Enter URL…" bar near the top right
   of the screen with the pointer, over whatever is in front (also while the
   browser window is hidden). Its suggestions come from Rust (open tabs and
   favourites, eclipse_show_little_command_bar); typing filters them and
   adds the typed text as the first row. ↑/↓ choose, Return opens the chosen
   row in a new standalone Little Eclipse window (Rust resolves typed text to
   an address or a search: "little_command_bar_open:<text>"), and Escape,
   the × button or a click anywhere else closes the bar. */

@class EclipseCommandBar;

static const CGFloat kEclipseCommandBarWidth = 560.0;
static const CGFloat kEclipseCommandBarSearchHeight = 52.0;
static const CGFloat kEclipseCommandBarRowHeight = 38.0;
static const NSUInteger kEclipseCommandBarMaxRows = 7;
static const NSUInteger kEclipseCommandBarMaxSuggestions = 64;

/* A borderless panel that still takes the keyboard. */
@interface EclipseCommandBarPanel : NSPanel
@end

@implementation EclipseCommandBarPanel
- (BOOL)canBecomeKeyWindow {
    return YES;
}
- (BOOL)canBecomeMainWindow {
    return NO;
}
@end

@interface EclipseCommandBarContentView : NSView
@end

@implementation EclipseCommandBarContentView
- (BOOL)isFlipped {
    return YES;
}
@end

/* One suggestion row: an icon (the site's initial on a tile coloured from
   its host, or a symbol for the typed text) and "Title — host". */
@interface EclipseCommandBarRow : NSView
@property(nonatomic, weak) EclipseCommandBar *owner;
@property(nonatomic) NSUInteger index;
@property(nonatomic, strong) NSTextField *label;
@property(nonatomic, strong) NSView *tile;
@property(nonatomic, strong) NSTextField *initial;
@property(nonatomic, strong) NSImageView *symbol;
@property(nonatomic, copy) NSString *title;
@property(nonatomic, copy) NSString *detail;
/* What Return opens: a suggestion's URL or the typed text. */
@property(nonatomic, copy) NSString *value;
@property(nonatomic) BOOL selected;
@end

@interface EclipseCommandBar : NSObject <NSTextFieldDelegate, NSWindowDelegate>
@property(nonatomic, strong) EclipseCommandBarPanel *panel;
@property(nonatomic, strong) NSView *tint;
@property(nonatomic, strong) NSTextField *field;
@property(nonatomic, strong) NSButton *closeButton;
@property(nonatomic, strong) NSView *rowsView;
@property(nonatomic, strong) NSMutableArray<EclipseCommandBarRow *> *rows;
/* From Rust: @[title, url] pairs. */
@property(nonatomic, copy) NSArray<NSArray<NSString *> *> *suggestions;
@property(nonatomic, strong) NSColor *accent;
@property(nonatomic) NSUInteger selected;
/* The window that was key before the bar (menu commands go to it). */
@property(nonatomic, weak) NSWindow *previousKey;
@property(nonatomic) BOOL closing;
- (void)selectRow:(NSUInteger)index;
- (void)openRow:(NSUInteger)index;
- (void)close;
@end

static EclipseCommandBar *gCommandBar = nil;

/* A short host for a suggestion's second part ("www." dropped). */
static NSString *eclipse_command_bar_host(NSString *urlString) {
    NSString *host = [NSURL URLWithString:urlString].host.lowercaseString ?: @"";
    if ([host hasPrefix:@"www."]) host = [host substringFromIndex:4];
    return host;
}

/* Whether typed text looks like an address rather than a search (for the
   label only: Rust decides what it opens). */
static BOOL eclipse_command_bar_looks_like_address(NSString *text) {
    if (text.length == 0 || [text rangeOfCharacterFromSet:[NSCharacterSet whitespaceCharacterSet]].location != NSNotFound) {
        return NO;
    }
    return [text containsString:@"."] || [text containsString:@"://"] || [text hasPrefix:@"localhost"];
}

@implementation EclipseCommandBarRow

- (instancetype)initWithFrame:(NSRect)frame {
    if ((self = [super initWithFrame:frame])) {
        self.wantsLayer = YES;
        self.layer.cornerRadius = 8.0;
        if (@available(macOS 10.15, *)) self.layer.cornerCurve = kCACornerCurveContinuous;
        _tile = [[NSView alloc] initWithFrame:NSMakeRect(12, floor((NSHeight(frame) - 20) / 2), 20, 20)];
        _tile.wantsLayer = YES;
        _tile.layer.cornerRadius = 5.0;
        [self addSubview:_tile];
        _initial = [NSTextField labelWithString:@""];
        _initial.font = [NSFont systemFontOfSize:11 weight:NSFontWeightBold];
        _initial.textColor = [NSColor whiteColor];
        _initial.alignment = NSTextAlignmentCenter;
        _initial.frame = NSMakeRect(0, 3, 20, 14);
        [_tile addSubview:_initial];
        _symbol = [[NSImageView alloc] initWithFrame:_tile.frame];
        _symbol.hidden = YES;
        [self addSubview:_symbol];
        _label = [NSTextField labelWithString:@""];
        _label.lineBreakMode = NSLineBreakByTruncatingTail;
        _label.cell.truncatesLastVisibleLine = YES;
        _label.maximumNumberOfLines = 1;
        [self addSubview:_label];
        [self addTrackingArea:[[NSTrackingArea alloc] initWithRect:NSZeroRect
                                                           options:NSTrackingMouseEnteredAndExited |
                                                                   NSTrackingActiveAlways | NSTrackingInVisibleRect
                                                             owner:self
                                                          userInfo:nil]];
    }
    return self;
}

- (BOOL)isFlipped {
    return YES;
}

/* The whole row takes the click, not its label. */
- (NSView *)hitTest:(NSPoint)point {
    return NSPointInRect([self convertPoint:point fromView:self.superview], self.bounds) ? self : nil;
}

- (void)mouseEntered:(NSEvent *)event {
    (void)event;
    [self.owner selectRow:self.index];
}

- (void)mouseDown:(NSEvent *)event {
    (void)event;
    [self.owner openRow:self.index];
}

- (void)layout {
    [super layout];
    CGFloat height = NSHeight(self.bounds);
    self.tile.frame = NSMakeRect(12, floor((height - 20) / 2), 20, 20);
    self.symbol.frame = self.tile.frame;
    CGFloat labelHeight = 18.0;
    self.label.frame = NSMakeRect(42, floor((height - labelHeight) / 2), MAX(0.0, NSWidth(self.bounds) - 54), labelHeight);
}

/* Sets the row's content; `typed` rows show a symbol, others a tile. */
- (void)setTitle:(NSString *)title detail:(NSString *)detail value:(NSString *)value typed:(BOOL)typed {
    self.title = title;
    self.detail = detail;
    self.value = value;
    self.symbol.hidden = !typed;
    self.tile.hidden = typed;
    if (typed) {
        NSArray<NSString *> *names = eclipse_command_bar_looks_like_address(value) ? @[ @"globe" ] : @[ @"magnifyingglass" ];
        self.symbol.image = eclipse_little_symbol(names, detail);
    } else {
        NSString *host = eclipse_command_bar_host(value);
        NSString *letter = host.length > 0 ? [host substringToIndex:[host rangeOfComposedCharacterSequenceAtIndex:0].length]
                                            : @"•";
        self.initial.stringValue = letter.uppercaseString;
        // A steady colour per site.
        NSUInteger hash = 5381;
        for (NSUInteger i = 0; i < host.length; i++) hash = hash * 33 + [host characterAtIndex:i];
        self.tile.layer.backgroundColor =
            [NSColor colorWithCalibratedHue:(CGFloat)(hash % 360) / 360.0 saturation:0.55 brightness:0.78 alpha:1.0]
                .CGColor;
    }
    [self refresh];
}

- (void)setSelected:(BOOL)selected {
    _selected = selected;
    [self refresh];
}

- (void)refresh {
    NSColor *accent = self.owner.accent ?: [NSColor controlAccentColor];
    self.layer.backgroundColor = self.selected ? accent.CGColor : [NSColor clearColor].CGColor;
    NSColor *titleColor = self.selected ? [NSColor whiteColor] : [NSColor labelColor];
    NSColor *detailColor = self.selected ? [NSColor colorWithCalibratedWhite:1.0 alpha:0.72] : [NSColor secondaryLabelColor];
    self.symbol.contentTintColor = self.selected ? [NSColor whiteColor] : [NSColor secondaryLabelColor];
    NSMutableParagraphStyle *style = [[NSMutableParagraphStyle alloc] init];
    style.lineBreakMode = NSLineBreakByTruncatingTail;
    NSMutableAttributedString *text = [[NSMutableAttributedString alloc]
        initWithString:self.title ?: @""
            attributes:@{NSFontAttributeName : [NSFont systemFontOfSize:13 weight:NSFontWeightMedium],
                         NSForegroundColorAttributeName : titleColor,
                         NSParagraphStyleAttributeName : style}];
    if (self.detail.length > 0) {
        [text appendAttributedString:[[NSAttributedString alloc]
                                         initWithString:[@" — " stringByAppendingString:self.detail]
                                             attributes:@{NSFontAttributeName : [NSFont systemFontOfSize:13],
                                                          NSForegroundColorAttributeName : detailColor,
                                                          NSParagraphStyleAttributeName : style}]];
    }
    self.label.attributedStringValue = text;
}

@end

@implementation EclipseCommandBar

/* The rows for what is typed: the text itself first, then the suggestions
   whose title or address contains it (all of them while nothing is typed). */
- (void)refilter {
    NSString *text = [self.field.stringValue
        stringByTrimmingCharactersInSet:[NSCharacterSet whitespaceAndNewlineCharacterSet]];
    NSMutableArray<NSArray *> *items = [NSMutableArray array];
    if (text.length > 0) {
        NSString *what = eclipse_command_bar_looks_like_address(text) ? @"Open" : @"Search";
        [items addObject:@[ text, what, text, @YES ]];
    }
    for (NSArray<NSString *> *suggestion in self.suggestions) {
        if (items.count >= kEclipseCommandBarMaxRows) break;
        NSString *title = suggestion[0], *url = suggestion[1];
        if (text.length > 0 && [title rangeOfString:text options:NSCaseInsensitiveSearch].location == NSNotFound &&
            [url rangeOfString:text options:NSCaseInsensitiveSearch].location == NSNotFound) {
            continue;
        }
        NSString *host = eclipse_command_bar_host(url);
        [items addObject:@[ title.length > 0 ? title : host, host, url, @NO ]];
    }

    for (EclipseCommandBarRow *row in self.rows) [row removeFromSuperview];
    [self.rows removeAllObjects];
    CGFloat width = kEclipseCommandBarWidth - 16.0;
    for (NSUInteger i = 0; i < items.count; i++) {
        NSArray *item = items[i];
        EclipseCommandBarRow *row = [[EclipseCommandBarRow alloc]
            initWithFrame:NSMakeRect(0, i * kEclipseCommandBarRowHeight, width, kEclipseCommandBarRowHeight)];
        row.owner = self;
        row.index = i;
        row.identifier = [NSString stringWithFormat:@"eclipse-commandbar-row-%lu", (unsigned long)i];
        [row setTitle:item[0] detail:item[1] value:item[2] typed:[item[3] boolValue]];
        [self.rowsView addSubview:row];
        [self.rows addObject:row];
    }
    self.selected = 0;
    for (EclipseCommandBarRow *row in self.rows) row.selected = row.index == 0;
    [self resize];
}

/* The panel's height for its rows; its top edge stays where it is. */
- (void)resize {
    CGFloat rowsHeight = self.rows.count * kEclipseCommandBarRowHeight;
    CGFloat height = kEclipseCommandBarSearchHeight + (rowsHeight > 0 ? rowsHeight + 10.0 : 0.0);
    self.rowsView.frame = NSMakeRect(8, kEclipseCommandBarSearchHeight, kEclipseCommandBarWidth - 16.0, rowsHeight);
    NSRect frame = self.panel.frame;
    CGFloat top = NSMaxY(frame);
    frame.size.height = height;
    frame.origin.y = top - height;
    [self.panel setFrame:frame display:YES];
    [self.panel invalidateShadow];
}

- (void)selectRow:(NSUInteger)index {
    if (index >= self.rows.count) return;
    self.selected = index;
    for (EclipseCommandBarRow *row in self.rows) row.selected = row.index == index;
}

- (void)openRow:(NSUInteger)index {
    if (index >= self.rows.count || self.closing) return;
    NSString *value = self.rows[index].value;
    [self close];
    if (value.length == 0 || value.length > kEclipseLittleMaxAddressLength || gCallback == NULL) {
        NSBeep();
        return;
    }
    NSString *command = [@"little_command_bar_open:" stringByAppendingString:value];
    gCallback(command.UTF8String);
}

- (void)closeClicked:(id)sender {
    (void)sender;
    [self close];
}

- (void)close {
    if (self.closing) return;
    self.closing = YES;
    self.field.delegate = nil;
    self.panel.delegate = nil;
    self.closeButton.target = nil;
    [self.panel orderOut:nil];
    if (eclipse_perf_enabled()) fprintf(stderr, "[little] command bar closed\n");
    // Released on the next turn of the run loop: AppKit may still be
    // delivering the event that closed it.
    EclipseCommandBar *me = self;
    dispatch_async(dispatch_get_main_queue(), ^{
        if (gCommandBar == me) gCommandBar = nil;
    });
}

/* NSTextFieldDelegate */

- (void)controlTextDidChange:(NSNotification *)notification {
    (void)notification;
    [self refilter];
}

- (BOOL)control:(NSControl *)control textView:(NSTextView *)textView doCommandBySelector:(SEL)selector {
    (void)control; (void)textView;
    if (selector == @selector(moveDown:)) {
        if (self.rows.count > 0) [self selectRow:(self.selected + 1) % self.rows.count];
        return YES;
    }
    if (selector == @selector(moveUp:)) {
        if (self.rows.count > 0) [self selectRow:(self.selected + self.rows.count - 1) % self.rows.count];
        return YES;
    }
    if (selector == @selector(insertNewline:)) {
        if (self.rows.count > 0) [self openRow:self.selected];
        else NSBeep();
        return YES;
    }
    if (selector == @selector(cancelOperation:)) {
        [self close];
        return YES;
    }
    return NO;
}

/* NSWindowDelegate: a click anywhere else (another window, another app). */
- (void)windowDidResignKey:(NSNotification *)notification {
    (void)notification;
    [self close];
}

@end

static BOOL eclipse_is_command_bar_window(NSWindow *window) {
    return window != nil && gCommandBar != nil && !gCommandBar.closing && gCommandBar.panel == window;
}

/* Closes the bar for a menu command and returns the window that was key
   before it, which the command is for. */
static NSWindow *eclipse_command_bar_dismiss_for_command(void) {
    NSWindow *previous = gCommandBar.previousKey;
    [gCommandBar close];
    return previous;
}

/* Shows the bar (or refreshes the one showing) with `count` suggestions
   (`titles[i]`, `urls[i]`: UTF-8, http(s) addresses only) and the Space's
   accent colour for the chosen row. 1 when it shows. */
int eclipse_show_little_command_bar(const char *const *titles, const char *const *urls, size_t count, double red,
                                    double green, double blue) {
    @autoreleasepool {
        NSMutableArray<NSArray<NSString *> *> *suggestions = [NSMutableArray array];
        for (size_t i = 0; i < count && suggestions.count < kEclipseCommandBarMaxSuggestions; i++) {
            if (titles == NULL || urls == NULL || urls[i] == NULL) break;
            NSURL *url = eclipse_little_loadable_url(urls[i]);
            if (url == nil) continue;
            NSString *title = titles[i] != NULL ? [NSString stringWithUTF8String:titles[i]] : @"";
            [suggestions addObject:@[ eclipse_little_clean_title(title ?: @""), url.absoluteString ]];
        }
        NSColor *accent = nil;
        if (isfinite(red) && isfinite(green) && isfinite(blue)) {
            accent = [NSColor colorWithSRGBRed:MIN(MAX(red, 0.0), 1.0)
                                         green:MIN(MAX(green, 0.0), 1.0)
                                          blue:MIN(MAX(blue, 0.0), 1.0)
                                         alpha:1.0];
        }

        EclipseCommandBar *bar = gCommandBar;
        if (bar == nil || bar.closing) {
            bar = [[EclipseCommandBar alloc] init];
            bar.rows = [NSMutableArray array];
            bar.previousKey = eclipse_shortcut_window();
            NSScreen *screen = eclipse_screen_with_mouse();
            if (screen == nil) return 0;
            NSRect visible = screen.visibleFrame;
            NSRect frame = NSMakeRect(NSMaxX(visible) - kEclipseCommandBarWidth - 16.0,
                                      NSMaxY(visible) - 16.0 - kEclipseCommandBarSearchHeight,
                                      kEclipseCommandBarWidth, kEclipseCommandBarSearchHeight);
            frame = eclipse_clamp_frame(frame, visible);
            EclipseCommandBarPanel *panel =
                [[EclipseCommandBarPanel alloc] initWithContentRect:frame
                                                          styleMask:NSWindowStyleMaskBorderless
                                                            backing:NSBackingStoreBuffered
                                                              defer:NO];
            panel.releasedWhenClosed = NO;
            panel.opaque = NO;
            panel.backgroundColor = [NSColor clearColor];
            panel.hasShadow = YES;
            panel.level = NSFloatingWindowLevel;
            panel.hidesOnDeactivate = NO;
            panel.collectionBehavior = NSWindowCollectionBehaviorMoveToActiveSpace |
                                       NSWindowCollectionBehaviorFullScreenAuxiliary |
                                       NSWindowCollectionBehaviorTransient;
            panel.appearance = [NSAppearance appearanceNamed:gLittleFrameDark ? NSAppearanceNameDarkAqua
                                                                               : NSAppearanceNameAqua];
            panel.title = @"New Little Eclipse Window";

            NSVisualEffectView *effect = [[NSVisualEffectView alloc] initWithFrame:NSMakeRect(0, 0, NSWidth(frame), NSHeight(frame))];
            effect.material = NSVisualEffectMaterialPopover;
            effect.blendingMode = NSVisualEffectBlendingModeBehindWindow;
            effect.state = NSVisualEffectStateActive;
            effect.wantsLayer = YES;
            effect.layer.cornerRadius = 14.0;
            if (@available(macOS 10.15, *)) effect.layer.cornerCurve = kCACornerCurveContinuous;
            effect.layer.masksToBounds = YES;
            effect.layer.borderWidth = 1.0;
            effect.layer.borderColor = (gLittleFrameDark ? [NSColor colorWithCalibratedWhite:1.0 alpha:0.14]
                                                         : [NSColor colorWithCalibratedWhite:0.0 alpha:0.12])
                                           .CGColor;
            effect.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
            panel.contentView = effect;

            // The Space's colour over the blur, like the Little Eclipse frame.
            EclipseCommandBarContentView *content = [[EclipseCommandBarContentView alloc] initWithFrame:effect.bounds];
            content.wantsLayer = YES;
            content.layer.backgroundColor = [eclipse_little_frame_color() colorWithAlphaComponent:0.82].CGColor;
            content.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
            [effect addSubview:content];
            bar.tint = content;

            NSImageView *magnifier = [[NSImageView alloc] initWithFrame:NSMakeRect(18, 17, 18, 18)];
            magnifier.image = eclipse_little_symbol(@[ @"magnifyingglass" ], @"Search");
            magnifier.contentTintColor = [NSColor secondaryLabelColor];
            [content addSubview:magnifier];

            NSTextField *field = [[NSTextField alloc] initWithFrame:NSMakeRect(44, 14, NSWidth(frame) - 44 - 52, 24)];
            field.identifier = @"eclipse-commandbar-field";
            field.bezeled = NO;
            field.bordered = NO;
            field.drawsBackground = NO;
            field.focusRingType = NSFocusRingTypeNone;
            field.font = [NSFont systemFontOfSize:18];
            field.usesSingleLineMode = YES;
            field.cell.scrollable = YES;
            field.cell.wraps = NO;
            field.placeholderString = @"Search or Enter URL…";
            field.delegate = bar;
            [content addSubview:field];
            bar.field = field;

            NSButton *close = [[NSButton alloc] initWithFrame:NSMakeRect(NSWidth(frame) - 42, 13, 26, 26)];
            close.identifier = @"eclipse-commandbar-close";
            close.bordered = NO;
            close.wantsLayer = YES;
            close.layer.cornerRadius = 6.0;
            close.layer.backgroundColor = (gLittleFrameDark ? [NSColor colorWithCalibratedWhite:1.0 alpha:0.08]
                                                            : [NSColor colorWithCalibratedWhite:0.0 alpha:0.06])
                                              .CGColor;
            close.refusesFirstResponder = YES;
            close.image = eclipse_little_symbol(@[ @"xmark" ], @"Close");
            if (close.image != nil) {
                close.imagePosition = NSImageOnly;
            } else {
                close.title = @"×";
            }
            close.contentTintColor = [NSColor secondaryLabelColor];
            close.toolTip = @"Close (Esc)";
            close.target = bar;
            close.action = @selector(closeClicked:);
            [content addSubview:close];
            bar.closeButton = close;

            NSView *rowsView = [[EclipseCommandBarContentView alloc] initWithFrame:NSZeroRect];
            [content addSubview:rowsView];
            bar.rowsView = rowsView;

            panel.delegate = bar;
            bar.panel = panel;
            gCommandBar = bar;
        }
        bar.accent = accent;
        bar.suggestions = suggestions;
        [bar refilter];
        [NSApp activateIgnoringOtherApps:YES];
        [bar.panel makeKeyAndOrderFront:nil];
        [bar.panel makeFirstResponder:bar.field];
        [bar.field selectText:nil];
        if (eclipse_perf_enabled()) {
            fprintf(stderr, "[little] command bar shown suggestions=%lu\n", (unsigned long)suggestions.count);
        }
        return 1;
    }
}
