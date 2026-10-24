#import <Cocoa/Cocoa.h>
#import <WebKit/WebKit.h>
#include <mach/mach.h>

static NSString *RawAssetSidebarHTML(void) {
    static NSString *s = nil;
    static dispatch_once_t onceToken;
    dispatch_once(&onceToken, ^{
        const char *raw =
        #include "../assets/generated_sidebar_cstr.h"
        ;
        s = [[NSString alloc] initWithUTF8String:raw];
    });
    return s;
}

static NSString *RawAssetNewtabHTML(void) {
    static NSString *s = nil;
    static dispatch_once_t onceToken;
    dispatch_once(&onceToken, ^{
        const char *raw =
        #include "../assets/generated_newtab_cstr.h"
        ;
        s = [[NSString alloc] initWithUTF8String:raw];
    });
    return s;
}

static NSString *RawAssetSettingsHTML(void) {
    static NSString *s = nil;
    static dispatch_once_t onceToken;
    dispatch_once(&onceToken, ^{
        const char *raw =
        #include "../assets/generated_settings_cstr.h"
        ;
        s = [[NSString alloc] initWithUTF8String:raw];
    });
    return s;
}

static double GetProcessMemoryMB(void) {
    struct mach_task_basic_info info;
    mach_msg_type_number_t count = MACH_TASK_BASIC_INFO_COUNT;
    if (task_info(mach_task_self(), MACH_TASK_BASIC_INFO, (task_info_t)&info, &count) == KERN_SUCCESS) {
        return (double)info.resident_size / (1024.0 * 1024.0);
    }
    return 42.0;
}

static NSString *GetDefaultFaviconType(NSString *urlStr) {
    if (!urlStr || [urlStr hasPrefix:@"eclipse://"]) return @"eclipse";
    NSString *lower = [urlStr lowercaseString];
    if ([lower containsString:@"netflix.com"]) return @"netflix";
    if ([lower containsString:@"youtube.com"] || [lower containsString:@"youtu.be"]) return @"youtube";
    if ([lower containsString:@"github.com"]) return @"github";
    if ([lower containsString:@"reddit.com"]) return @"reddit";
    if ([lower containsString:@"apple.com"]) return @"apple";
    if ([lower containsString:@"spotify.com"]) return @"spotify";
    return @"globe";
}

static NSString *GetFaviconUrl(NSString *urlStr) {
    if (!urlStr || [urlStr hasPrefix:@"eclipse://"]) return nil;
    NSURL *u = [NSURL URLWithString:urlStr];
    NSString *host = u.host;
    if (host.length > 0 && ![host isEqualToString:@"localhost"]) {
        return [NSString stringWithFormat:@"https://icons.duckduckgo.com/ip3/%@.ico", host];
    }
    return nil;
}

static NSString *CanonicalizeEclipseUrl(NSString *urlStr) {
    if (!urlStr || urlStr.length == 0) return @"eclipse://newtab";
    if ([urlStr hasPrefix:@"eclipse://"]) {
        if ([urlStr containsString:@"settings"]) return @"eclipse://settings";
        if ([urlStr containsString:@"sidebar"]) return @"eclipse://sidebar";
        return @"eclipse://newtab";
    }
    return urlStr;
}

@interface FlippedView : NSView
@end
@implementation FlippedView
- (BOOL)isFlipped { return YES; }
@end

@interface EclipseSchemeHandler : NSObject <WKURLSchemeHandler>
@end
@implementation EclipseSchemeHandler
- (void)webView:(WKWebView *)webView startURLSchemeTask:(id<WKURLSchemeTask>)urlSchemeTask {
    NSURL *url = urlSchemeTask.request.URL;
    NSString *host = url.host ?: @"";
    NSString *path = [url.path stringByTrimmingCharactersInSet:[NSCharacterSet characterSetWithCharactersInString:@"/"]] ?: @"";
    NSString *route = (path.length > 0 && ![path isEqualToString:@"localhost"]) ? path : host;

    NSString *html = nil;
    if ([route isEqualToString:@"sidebar"]) {
        html = RawAssetSidebarHTML();
    } else if ([route isEqualToString:@"settings"]) {
        html = RawAssetSettingsHTML();
    } else {
        html = RawAssetNewtabHTML();
    }

    NSData *data = [html dataUsingEncoding:NSUTF8StringEncoding] ?: [NSData data];
    NSDictionary *headers = @{
        @"Content-Type": @"text/html; charset=utf-8",
        @"Content-Length": [NSString stringWithFormat:@"%lu", (unsigned long)data.length],
        @"Access-Control-Allow-Origin": @"*"
    };
    NSHTTPURLResponse *response = [[NSHTTPURLResponse alloc] initWithURL:url
                                                              statusCode:200
                                                             HTTPVersion:@"HTTP/1.1"
                                                            headerFields:headers];
    [urlSchemeTask didReceiveResponse:response];
    [urlSchemeTask didReceiveData:data];
    [urlSchemeTask didFinish];
}
- (void)webView:(WKWebView *)webView stopURLSchemeTask:(id<WKURLSchemeTask>)urlSchemeTask {
}
@end

@class BrowserController;

@interface TabItem : NSObject <WKNavigationDelegate, WKUIDelegate, WKScriptMessageHandler>
@property (nonatomic, assign) NSUInteger tabId;
@property (nonatomic, copy) NSString *title;
@property (nonatomic, copy) NSString *url;
@property (nonatomic, copy) NSString *favicon;
@property (nonatomic, copy) NSString *faviconUrl;
@property (nonatomic, assign) BOOL isLoading;
@property (nonatomic, assign) BOOL isPinned;
@property (nonatomic, assign) BOOL isHibernated;
@property (nonatomic, copy) NSString *workspace;
@property (nonatomic, strong) WKWebView *webview;
@property (nonatomic, strong) NSDate *lastActive;
@property (nonatomic, assign) double scrollX;
@property (nonatomic, assign) double scrollY;
@property (nonatomic, assign) BOOL hasPendingScrollRestore;
@property (nonatomic, strong) NSMutableArray<NSString *> *history;
@property (nonatomic, assign) NSUInteger historyIndex;
@property (nonatomic, weak) BrowserController *controller;
- (void)recordNavigation:(NSString *)urlStr;
@end

@interface BrowserController : NSObject <NSApplicationDelegate, NSWindowDelegate, WKScriptMessageHandler>
@property (nonatomic, strong) NSWindow *window;
@property (nonatomic, strong) FlippedView *rootView;
@property (nonatomic, strong) NSVisualEffectView *sidebarEffectView;
@property (nonatomic, strong) WKWebView *sidebarWebView;
@property (nonatomic, strong) EclipseSchemeHandler *schemeHandler;
@property (nonatomic, strong) NSMutableArray<TabItem *> *tabs;
@property (nonatomic, assign) NSUInteger activeTabId;
@property (nonatomic, assign) NSUInteger nextTabId;
@property (nonatomic, copy) NSString *currentWorkspace;
@property (nonatomic, strong) NSMutableDictionary *config;
@property (nonatomic, assign) double lastBroadcastMemoryMB;
@property (nonatomic, strong) NSTimer *maintenanceTimer;

- (void)handleIpcDictionary:(NSDictionary *)cmd fromTab:(TabItem *)sourceTab;
- (void)broadcastState;
- (void)handlePageStarted:(TabItem *)tab url:(NSString *)urlStr;
- (void)handlePageFinished:(TabItem *)tab url:(NSString *)urlStr;
- (void)handleTitleChanged:(TabItem *)tab title:(NSString *)title;
- (NSUInteger)createTabWithUrl:(NSString *)url select:(BOOL)select;
- (WKWebView *)buildWebViewForTab:(TabItem *)tab frame:(NSRect)frame;
@end

@implementation TabItem

- (void)recordNavigation:(NSString *)urlStr {
    NSString *canonical = CanonicalizeEclipseUrl(urlStr);
    if (self.history.count == 0) {
        [self.history addObject:canonical];
        self.historyIndex = 0;
        return;
    }
    NSString *current = self.history[self.historyIndex];
    if (![current isEqualToString:canonical]) {
        if (self.historyIndex + 1 < self.history.count) {
            NSRange removeRange = NSMakeRange(self.historyIndex + 1, self.history.count - (self.historyIndex + 1));
            [self.history removeObjectsInRange:removeRange];
        }
        [self.history addObject:canonical];
        self.historyIndex = self.history.count - 1;
    }
}

- (void)userContentController:(WKUserContentController *)userContentController didReceiveScriptMessage:(WKScriptMessage *)message {
    if (![message.name isEqualToString:@"ipc"]) return;
    NSString *body = [message.body isKindOfClass:[NSString class]] ? message.body : [message.body description];
    NSData *data = [body dataUsingEncoding:NSUTF8StringEncoding];
    if (!data) return;
    NSDictionary *dict = [NSJSONSerialization JSONObjectWithData:data options:0 error:nil];
    if ([dict isKindOfClass:[NSDictionary class]]) {
        [self.controller handleIpcDictionary:dict fromTab:self];
    }
}

- (void)webView:(WKWebView *)webView didStartProvisionalNavigation:(WKNavigation *)navigation {
    NSString *u = webView.URL.absoluteString ?: self.url;
    [self.controller handlePageStarted:self url:u];
}

- (void)webView:(WKWebView *)webView didFinishNavigation:(WKNavigation *)navigation {
    NSString *u = webView.URL.absoluteString ?: self.url;
    [self.controller handlePageFinished:self url:u];
    __weak TabItem *weakSelf = self;
    [webView evaluateJavaScript:@"document.title" completionHandler:^(id result, NSError *error) {
        if ([result isKindOfClass:[NSString class]] && [(NSString *)result length] > 0) {
            [weakSelf.controller handleTitleChanged:weakSelf title:(NSString *)result];
        }
    }];
}

- (WKWebView *)webView:(WKWebView *)webView createWebViewWithConfiguration:(WKWebViewConfiguration *)configuration forNavigationAction:(WKNavigationAction *)navigationAction windowFeatures:(WKWindowFeatures *)windowFeatures {
    if (!navigationAction.targetFrame || !navigationAction.targetFrame.isMainFrame) {
        NSString *targetUrl = navigationAction.request.URL.absoluteString;
        if (targetUrl.length > 0) {
            [self.controller createTabWithUrl:targetUrl select:YES];
        }
    }
    return nil;
}

@end

@implementation BrowserController

- (NSString *)configFilePath {
    NSString *home = NSHomeDirectory();
    NSString *dir = [home stringByAppendingPathComponent:@".config/eclipse-browser"];
    [[NSFileManager defaultManager] createDirectoryAtPath:dir withIntermediateDirectories:YES attributes:nil error:nil];
    return [dir stringByAppendingPathComponent:@"config.json"];
}

- (NSMutableArray *)defaultFavorites {
    return [@[
        [@{ @"title": @"Apple", @"url": @"https://www.apple.com", @"icon": @"apple" } mutableCopy],
        [@{ @"title": @"GitHub", @"url": @"https://github.com", @"icon": @"github" } mutableCopy],
        [@{ @"title": @"YouTube", @"url": @"https://www.youtube.com", @"icon": @"youtube" } mutableCopy],
        [@{ @"title": @"Netflix", @"url": @"https://www.netflix.com", @"icon": @"netflix" } mutableCopy]
    ] mutableCopy];
}

- (NSMutableDictionary *)defaultConfig {
    return [@{
        @"theme": @"eclipse",
        @"accent_color": @"#5E9EFF",
        @"sidebar_width": @(232.0),
        @"sidebar_collapsed": @NO,
        @"search_engine": @"duckduckgo",
        @"user_agent": @"Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.6 Safari/605.1.15",
        @"drm_fairplay_enabled": @YES,
        @"autoplay_enabled": @YES,
        @"tab_hibernation_mins": @(15),
        @"adblock_enabled": @YES,
        @"custom_css": @"",
        @"custom_user_agent": @"",
        @"favorites": [self defaultFavorites]
    } mutableCopy];
}

- (void)loadConfig {
    NSMutableDictionary *defaults = [self defaultConfig];
    NSData *data = [NSData dataWithContentsOfFile:[self configFilePath]];
    if (data) {
        NSDictionary *loaded = [NSJSONSerialization JSONObjectWithData:data options:NSJSONReadingMutableContainers error:nil];
        if ([loaded isKindOfClass:[NSDictionary class]]) {
            [defaults addEntriesFromDictionary:loaded];
            NSArray *favs = defaults[@"favorites"];
            if (![favs isKindOfClass:[NSArray class]] || favs.count == 0) {
                defaults[@"favorites"] = [self defaultFavorites];
            }
        }
    }
    self.config = defaults;
}

- (void)saveConfig {
    if (!self.config) return;
    NSData *data = [NSJSONSerialization dataWithJSONObject:self.config options:NSJSONWritingPrettyPrinted error:nil];
    if (data) {
        [data writeToFile:[self configFilePath] atomically:YES];
    }
}

- (NSString *)normalizeUrl:(NSString *)input {
    NSString *trimmed = [input stringByTrimmingCharactersInSet:[NSCharacterSet whitespaceAndNewlineCharacterSet]];
    if (trimmed.length == 0) return @"eclipse://newtab";
    if ([trimmed hasPrefix:@"eclipse://"]) return CanonicalizeEclipseUrl(trimmed);
    if ([trimmed hasPrefix:@"http://"] || [trimmed hasPrefix:@"https://"]) return trimmed;
    if (![trimmed containsString:@" "] && ([trimmed containsString:@"."] || [trimmed hasPrefix:@"localhost"])) {
        if ([trimmed hasPrefix:@"localhost"]) return [@"http://" stringByAppendingString:trimmed];
        return [@"https://" stringByAppendingString:trimmed];
    }
    NSString *encoded = [trimmed stringByAddingPercentEncodingWithAllowedCharacters:[NSCharacterSet URLQueryAllowedCharacterSet]] ?: @"";
    NSString *engine = self.config[@"search_engine"] ?: @"duckduckgo";
    if ([engine isEqualToString:@"google"]) return [NSString stringWithFormat:@"https://www.google.com/search?q=%@", encoded];
    if ([engine isEqualToString:@"brave"]) return [NSString stringWithFormat:@"https://search.brave.com/search?q=%@", encoded];
    if ([engine isEqualToString:@"ecosia"]) return [NSString stringWithFormat:@"https://www.ecosia.org/search?q=%@", encoded];
    if ([engine isEqualToString:@"kagi"]) return [NSString stringWithFormat:@"https://kagi.com/search?q=%@", encoded];
    return [NSString stringWithFormat:@"https://duckduckgo.com/?q=%@", encoded];
}

- (CGFloat)sidebarWidth {
    if ([self.config[@"sidebar_collapsed"] boolValue]) {
        return 64.0;
    }
    double w = [self.config[@"sidebar_width"] doubleValue];
    if (w < 200.0 || w > 248.0) w = 232.0;
    return (CGFloat)w;
}

- (NSRect)sidebarBounds {
    NSRect b = self.rootView.bounds;
    return NSMakeRect(0, 0, [self sidebarWidth], MAX(100.0, b.size.height));
}

- (NSRect)contentBounds {
    NSRect b = self.rootView.bounds;
    CGFloat sw = [self sidebarWidth];
    return NSMakeRect(sw, 0, MAX(100.0, b.size.width - sw), MAX(100.0, b.size.height));
}

- (void)setupMainMenu {
    NSMenu *menubar = [[NSMenu alloc] init];
    [NSApp setMainMenu:menubar];

    // App Menu
    NSMenuItem *appMenuItem = [[NSMenuItem alloc] init];
    [menubar addItem:appMenuItem];
    NSMenu *appMenu = [[NSMenu alloc] initWithTitle:@"Eclipse"];
    [appMenu addItemWithTitle:@"About Eclipse" action:@selector(orderFrontStandardAboutPanel:) keyEquivalent:@""];
    [appMenu addItem:[NSMenuItem separatorItem]];
    NSMenuItem *prefsItem = [[NSMenuItem alloc] initWithTitle:@"Preferences…" action:@selector(menuOpenPreferences:) keyEquivalent:@","];
    prefsItem.target = self;
    [appMenu addItem:prefsItem];
    [appMenu addItem:[NSMenuItem separatorItem]];
    [appMenu addItemWithTitle:@"Hide Eclipse" action:@selector(hide:) keyEquivalent:@"h"];
    [appMenu addItemWithTitle:@"Quit Eclipse" action:@selector(terminate:) keyEquivalent:@"q"];
    [appMenuItem setSubmenu:appMenu];

    // File Menu
    NSMenuItem *fileMenuItem = [[NSMenuItem alloc] init];
    [menubar addItem:fileMenuItem];
    NSMenu *fileMenu = [[NSMenu alloc] initWithTitle:@"File"];
    NSMenuItem *newTabItem = [[NSMenuItem alloc] initWithTitle:@"New Tab" action:@selector(menuNewTab:) keyEquivalent:@"t"];
    newTabItem.target = self;
    [fileMenu addItem:newTabItem];
    NSMenuItem *focusUrlItem = [[NSMenuItem alloc] initWithTitle:@"Open Location…" action:@selector(menuFocusOmnibar:) keyEquivalent:@"l"];
    focusUrlItem.target = self;
    [fileMenu addItem:focusUrlItem];
    NSMenuItem *closeTabItem = [[NSMenuItem alloc] initWithTitle:@"Close Tab" action:@selector(menuCloseTab:) keyEquivalent:@"w"];
    closeTabItem.target = self;
    [fileMenu addItem:closeTabItem];
    [fileMenuItem setSubmenu:fileMenu];

    // Edit Menu (enables standard Cut/Copy/Paste/Select All in WKWebView inputs)
    NSMenuItem *editMenuItem = [[NSMenuItem alloc] init];
    [menubar addItem:editMenuItem];
    NSMenu *editMenu = [[NSMenu alloc] initWithTitle:@"Edit"];
    [editMenu addItemWithTitle:@"Undo" action:@selector(undo:) keyEquivalent:@"z"];
    [editMenu addItemWithTitle:@"Redo" action:@selector(redo:) keyEquivalent:@"Z"];
    [editMenu addItem:[NSMenuItem separatorItem]];
    [editMenu addItemWithTitle:@"Cut" action:@selector(cut:) keyEquivalent:@"x"];
    [editMenu addItemWithTitle:@"Copy" action:@selector(copy:) keyEquivalent:@"c"];
    [editMenu addItemWithTitle:@"Paste" action:@selector(paste:) keyEquivalent:@"v"];
    [editMenu addItemWithTitle:@"Select All" action:@selector(selectAll:) keyEquivalent:@"a"];
    [editMenuItem setSubmenu:editMenu];

    // View Menu
    NSMenuItem *viewMenuItem = [[NSMenuItem alloc] init];
    [menubar addItem:viewMenuItem];
    NSMenu *viewMenu = [[NSMenu alloc] initWithTitle:@"View"];
    NSMenuItem *toggleSidebarItem = [[NSMenuItem alloc] initWithTitle:@"Toggle Sidebar" action:@selector(menuToggleSidebar:) keyEquivalent:@"b"];
    toggleSidebarItem.target = self;
    [viewMenu addItem:toggleSidebarItem];
    NSMenuItem *reloadItem = [[NSMenuItem alloc] initWithTitle:@"Reload Page" action:@selector(menuReload:) keyEquivalent:@"r"];
    reloadItem.target = self;
    [viewMenu addItem:reloadItem];
    NSMenuItem *backItem = [[NSMenuItem alloc] initWithTitle:@"Back" action:@selector(menuGoBack:) keyEquivalent:@"["];
    backItem.target = self;
    [viewMenu addItem:backItem];
    NSMenuItem *fwdItem = [[NSMenuItem alloc] initWithTitle:@"Forward" action:@selector(menuGoForward:) keyEquivalent:@"]"];
    fwdItem.target = self;
    [viewMenu addItem:fwdItem];
    [viewMenuItem setSubmenu:viewMenu];
}

- (void)menuNewTab:(id)sender { [self createTabWithUrl:@"eclipse://newtab" select:YES]; }
- (void)menuCloseTab:(id)sender { [self closeTabWithId:self.activeTabId]; }
- (void)menuReload:(id)sender {
    TabItem *t = [self activeTab];
    if (t.webview) [t.webview evaluateJavaScript:@"location.reload();" completionHandler:nil];
}
- (void)menuFocusOmnibar:(id)sender { [self focusOmnibar]; }
- (void)menuToggleSidebar:(id)sender {
    BOOL collapsed = ![self.config[@"sidebar_collapsed"] boolValue];
    self.config[@"sidebar_collapsed"] = @(collapsed);
    [self saveConfig];
    [self updateLayout];
}
- (void)menuOpenPreferences:(id)sender { [self openSettingsDrawer]; }
- (void)menuGoBack:(id)sender { [self goBackActive]; }
- (void)menuGoForward:(id)sender { [self goForwardActive]; }

- (void)hideNativeTrafficLights {
    NSButton *closeBtn = [self.window standardWindowButton:NSWindowCloseButton];
    NSButton *minBtn = [self.window standardWindowButton:NSWindowMiniaturizeButton];
    NSButton *zoomBtn = [self.window standardWindowButton:NSWindowZoomButton];
    if (closeBtn) closeBtn.hidden = YES;
    if (minBtn) minBtn.hidden = YES;
    if (zoomBtn) zoomBtn.hidden = YES;
}

- (void)applicationDidFinishLaunching:(NSNotification *)notification {
    [self loadConfig];
    self.tabs = [NSMutableArray array];
    self.activeTabId = 1;
    self.nextTabId = 1;
    self.currentWorkspace = @"Default";
    self.lastBroadcastMemoryMB = GetProcessMemoryMB();
    self.schemeHandler = [[EclipseSchemeHandler alloc] init];

    [self setupMainMenu];

    NSRect initialRect = NSMakeRect(0, 0, 1340, 860);
    NSWindowStyleMask style = NSWindowStyleMaskTitled | NSWindowStyleMaskClosable | NSWindowStyleMaskMiniaturizable | NSWindowStyleMaskResizable | NSWindowStyleMaskFullSizeContentView;
    self.window = [[NSWindow alloc] initWithContentRect:initialRect styleMask:style backing:NSBackingStoreBuffered defer:NO];
    self.window.title = @"Eclipse";
    self.window.minSize = NSMakeSize(860, 560);
    self.window.titlebarAppearsTransparent = YES;
    self.window.titleVisibility = NSWindowTitleHidden;
    self.window.opaque = NO;
    self.window.backgroundColor = [NSColor colorWithCalibratedRed:0.04 green:0.04 blue:0.06 alpha:1.0];
    self.window.delegate = self;
    [self.window center];
    [self hideNativeTrafficLights];

    self.rootView = [[FlippedView alloc] initWithFrame:initialRect];
    self.rootView.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
    self.window.contentView = self.rootView;

    // Native macOS sidebar vibrancy layer
    self.sidebarEffectView = [[NSVisualEffectView alloc] initWithFrame:[self sidebarBounds]];
    self.sidebarEffectView.material = NSVisualEffectMaterialSidebar;
    self.sidebarEffectView.blendingMode = NSVisualEffectBlendingModeBehindWindow;
    self.sidebarEffectView.state = NSVisualEffectStateActive;
    [self.rootView addSubview:self.sidebarEffectView];

    // Sidebar WKWebView
    WKWebViewConfiguration *sidebarCfg = [[WKWebViewConfiguration alloc] init];
    [sidebarCfg setURLSchemeHandler:self.schemeHandler forURLScheme:@"eclipse"];
    WKUserContentController *ucc = [[WKUserContentController alloc] init];
    [ucc addScriptMessageHandler:self name:@"ipc"];
    WKUserScript *ipcShim = [[WKUserScript alloc] initWithSource:@"window.ipc = { postMessage: function(m) { window.webkit.messageHandlers.ipc.postMessage(String(m)); } };"
                                                   injectionTime:WKUserScriptInjectionTimeAtDocumentStart
                                                forMainFrameOnly:NO];
    [ucc addUserScript:ipcShim];
    sidebarCfg.userContentController = ucc;

    self.sidebarWebView = [[WKWebView alloc] initWithFrame:[self sidebarBounds] configuration:sidebarCfg];
    [self.sidebarWebView setValue:@NO forKey:@"drawsBackground"];
    [self.rootView addSubview:self.sidebarWebView];
    [self.sidebarWebView loadRequest:[NSURLRequest requestWithURL:[NSURL URLWithString:@"eclipse://localhost/sidebar"]]];

    // Create initial New Tab
    [self createTabWithUrl:@"eclipse://newtab" select:YES];
    [self broadcastState];

    [self.window makeKeyAndOrderFront:nil];
    [NSApp activateIgnoringOtherApps:YES];

    __weak BrowserController *weakSelf = self;
    self.maintenanceTimer = [NSTimer scheduledTimerWithTimeInterval:10.0 repeats:YES block:^(NSTimer * _Nonnull timer) {
        [weakSelf handleMaintenanceTick];
    }];
}

- (BOOL)applicationShouldTerminateAfterLastWindowClosed:(NSApplication *)sender {
    return YES;
}

- (void)windowDidResize:(NSNotification *)notification {
    [self hideNativeTrafficLights];
    [self updateLayout];
}

- (void)windowDidExitFullScreen:(NSNotification *)notification {
    [self hideNativeTrafficLights];
}

- (TabItem *)activeTab {
    for (TabItem *t in self.tabs) {
        if (t.tabId == self.activeTabId) return t;
    }
    return nil;
}

- (NSString *)buildCustomCssScript:(NSString *)css {
    NSData *jsonData = [NSJSONSerialization dataWithJSONObject:@[css ?: @""] options:0 error:nil];
    NSString *jsonArr = [[NSString alloc] initWithData:jsonData encoding:NSUTF8StringEncoding] ?: @"[\"\"]";
    return [NSString stringWithFormat:@"(function(){ const cssText = (%@)[0]; function applyUserCss(){ let el = document.getElementById('__eclipse_user_custom_css'); if (!cssText.trim()){ if(el) el.remove(); return; } if(!el){ el = document.createElement('style'); el.id = '__eclipse_user_custom_css'; (document.head || document.documentElement).appendChild(el); } el.textContent = cssText; } if(document.head || document.documentElement) applyUserCss(); else document.addEventListener('DOMContentLoaded', applyUserCss, {once:true}); })();", jsonArr];
}

- (WKWebView *)buildWebViewForTab:(TabItem *)tab frame:(NSRect)frame {
    WKWebViewConfiguration *cfg = [[WKWebViewConfiguration alloc] init];
    [cfg setURLSchemeHandler:self.schemeHandler forURLScheme:@"eclipse"];
    cfg.mediaTypesRequiringUserActionForPlayback = [self.config[@"autoplay_enabled"] boolValue] ? WKAudiovisualMediaTypeNone : WKAudiovisualMediaTypeAll;

    WKUserContentController *ucc = [[WKUserContentController alloc] init];
    [ucc addScriptMessageHandler:tab name:@"ipc"];

    NSData *cfgData = [NSJSONSerialization dataWithJSONObject:self.config options:0 error:nil];
    NSString *cfgJson = [[NSString alloc] initWithData:cfgData encoding:NSUTF8StringEncoding] ?: @"{}";

    NSMutableString *initScript = [NSMutableString string];
    [initScript appendFormat:@"window.ipc = { postMessage: function(m) { window.webkit.messageHandlers.ipc.postMessage(String(m)); } };\n"];
    [initScript appendFormat:@"window.__ECLIPSE_CONFIG__ = %@;\n", cfgJson];
    [initScript appendString:@"(function(){\n"
     "if (window.__eclipse_bridge_active) return; window.__eclipse_bridge_active = true;\n"
     "function postIpc(p){ if(window.ipc) window.ipc.postMessage(JSON.stringify(p)); }\n"
     "let scrollTimer = null;\n"
     "window.addEventListener('scroll', function(){ if(scrollTimer) clearTimeout(scrollTimer); scrollTimer = setTimeout(function(){ postIpc({action:'update_scroll', x:window.scrollX||0, y:window.scrollY||0}); }, 350); }, {passive:true});\n"
     "function detectFav(){ if(location.protocol==='eclipse:') return; const l = document.querySelector('link[rel*=\"icon\"][href]'); if(l && l.href && l.href.startsWith('http')) postIpc({action:'update_favicon', favicon:l.href}); }\n"
     "if(document.readyState==='loading') document.addEventListener('DOMContentLoaded', detectFav, {once:true}); else detectFav();\n"
     "})();\n"];

    if ([self.config[@"adblock_enabled"] boolValue]) {
        [initScript appendString:@"(function(){\n"
         "if(window.__eclipse_shields_active) return; window.__eclipse_shields_active = true;\n"
         "function injectCss(){ if(document.getElementById('__eclipse_adblock_css')) return; const s = document.createElement('style'); s.id='__eclipse_adblock_css'; s.textContent='.adsbygoogle,[id^=\"google_ads_\"],.ad-banner,.ad-unit,.advertisement,.ad-container,[data-ad-slot],.ytp-ad-overlay-container,.ytp-ad-message-container,ytd-promoted-sparkles-web-renderer,ytd-display-ad-renderer,ytd-banner-promo-renderer,ytd-ad-slot-renderer,#player-ads,div[id*=\"ad-slot\"],.outbrain,.taboola{display:none!important;}'; (document.head||document.documentElement).appendChild(s); }\n"
         "if(document.head||document.documentElement) injectCss(); else document.addEventListener('DOMContentLoaded', injectCss, {once:true});\n"
         "if(location.hostname.includes('youtube.com')){\n"
         "  let raf=false;\n"
         "  function skipAds(){ raf=false; const b=document.querySelector('.ytp-ad-skip-button,.ytp-skip-ad-button,.ytp-ad-skip-button-modern'); if(b) b.click(); const c=document.querySelector('.ytp-ad-overlay-close-button'); if(c) c.click(); const ad=document.querySelector('.ad-showing,.ad-interrupting'); if(ad){ const v=document.querySelector('video'); if(v && isFinite(v.duration) && v.duration>0) v.currentTime=v.duration; } }\n"
         "  const obs=new MutationObserver(function(){ if(!raf){ raf=true; requestAnimationFrame(skipAds); } });\n"
         "  function start(){ const t=document.getElementById('movie_player')||document.body||document.documentElement; if(t){ obs.observe(t,{childList:true,subtree:true,attributes:true,attributeFilter:['class']}); skipAds(); } }\n"
         "  if(document.readyState==='loading') document.addEventListener('DOMContentLoaded', start, {once:true}); else start();\n"
         "}\n"
         "})();\n"];
    }

    [initScript appendString:[self buildCustomCssScript:self.config[@"custom_css"]]];

    WKUserScript *userScript = [[WKUserScript alloc] initWithSource:initScript
                                                      injectionTime:WKUserScriptInjectionTimeAtDocumentStart
                                                   forMainFrameOnly:YES];
    [ucc addUserScript:userScript];
    cfg.userContentController = ucc;

    WKWebView *wv = [[WKWebView alloc] initWithFrame:frame configuration:cfg];
    wv.navigationDelegate = tab;
    wv.UIDelegate = tab;
    NSString *ua = self.config[@"custom_user_agent"];
    if (![ua isKindOfClass:[NSString class]] || ua.length == 0) {
        ua = self.config[@"user_agent"];
    }
    if ([ua isKindOfClass:[NSString class]] && ua.length > 0) {
        wv.customUserAgent = ua;
    }

    [self.rootView addSubview:wv];

    NSString *targetUrl = tab.url;
    if ([targetUrl isEqualToString:@"eclipse://newtab"] || targetUrl.length == 0) {
        targetUrl = @"eclipse://localhost/newtab";
    } else if ([targetUrl isEqualToString:@"eclipse://settings"]) {
        targetUrl = @"eclipse://localhost/settings";
    }
    [wv loadRequest:[NSURLRequest requestWithURL:[NSURL URLWithString:targetUrl]]];
    return wv;
}

- (NSUInteger)createTabWithUrl:(NSString *)url select:(BOOL)select {
    NSUInteger tid = self.nextTabId++;
    NSString *normalized = (url.length == 0) ? @"eclipse://newtab" : [self normalizeUrl:url];

    TabItem *tab = [[TabItem alloc] init];
    tab.tabId = tid;
    tab.url = normalized;
    if ([normalized isEqualToString:@"eclipse://settings"]) {
        tab.title = @"Preferences";
    } else if ([normalized hasPrefix:@"eclipse://"]) {
        tab.title = @"New Tab";
    } else {
        tab.title = @"Loading...";
    }
    tab.favicon = GetDefaultFaviconType(normalized);
    tab.faviconUrl = GetFaviconUrl(normalized);
    tab.isLoading = ![normalized hasPrefix:@"eclipse://"];
    tab.isPinned = NO;
    tab.isHibernated = NO;
    tab.workspace = self.currentWorkspace ?: @"Default";
    tab.lastActive = [NSDate date];
    tab.history = [NSMutableArray arrayWithObject:normalized];
    tab.historyIndex = 0;
    tab.controller = self;
    tab.webview = [self buildWebViewForTab:tab frame:[self contentBounds]];

    [self.tabs addObject:tab];
    if (select) {
        [self selectTabWithId:tid];
    } else {
        tab.webview.hidden = YES;
        [self broadcastState];
    }
    return tid;
}

- (void)selectTabWithId:(NSUInteger)tid {
    NSRect cBounds = [self contentBounds];
    NSString *targetWs = nil;

    for (TabItem *tab in self.tabs) {
        if (tab.tabId == tid) {
            if (tab.isHibernated || !tab.webview) {
                tab.hasPendingScrollRestore = (tab.scrollX > 0 || tab.scrollY > 0);
                tab.webview = [self buildWebViewForTab:tab frame:cBounds];
                tab.isHibernated = NO;
            }
            tab.lastActive = [NSDate date];
            tab.webview.frame = cBounds;
            tab.webview.hidden = NO;
            [self syncConfigToTab:tab];
            if (!tab.isPinned) {
                targetWs = tab.workspace;
            }
        } else {
            tab.webview.hidden = YES;
        }
    }
    if (targetWs) {
        self.currentWorkspace = targetWs;
    }
    self.activeTabId = tid;
    [self broadcastState];
}

- (void)closeTabWithId:(NSUInteger)tid {
    NSInteger idx = NSNotFound;
    for (NSUInteger i = 0; i < self.tabs.count; i++) {
        if (self.tabs[i].tabId == tid) {
            idx = i;
            break;
        }
    }
    if (idx == NSNotFound) return;

    TabItem *closing = self.tabs[idx];
    NSString *closedWs = closing.workspace ?: @"Default";
    [closing.webview removeFromSuperview];
    closing.webview = nil;
    [self.tabs removeObjectAtIndex:idx];

    NSMutableArray<TabItem *> *wsTabs = [NSMutableArray array];
    for (TabItem *t in self.tabs) {
        if ([t.workspace isEqualToString:closedWs]) {
            [wsTabs addObject:t];
        }
    }

    if (wsTabs.count == 0) {
        self.currentWorkspace = closedWs;
        [self createTabWithUrl:@"eclipse://newtab" select:YES];
    } else if (self.activeTabId == tid) {
        [self selectTabWithId:wsTabs.lastObject.tabId];
    } else {
        [self broadcastState];
    }
}

- (void)switchWorkspace:(NSString *)ws {
    self.currentWorkspace = ws;
    TabItem *active = [self activeTab];
    if (active && [active.workspace isEqualToString:ws]) {
        [self broadcastState];
        return;
    }
    TabItem *best = nil;
    for (TabItem *t in self.tabs) {
        if ([t.workspace isEqualToString:ws]) {
            if (!best || [t.lastActive compare:best.lastActive] == NSOrderedDescending) {
                best = t;
            }
        }
    }
    if (best) {
        [self selectTabWithId:best.tabId];
    } else {
        [self createTabWithUrl:@"eclipse://newtab" select:YES];
    }
}

- (void)navigateActive:(NSString *)input {
    NSString *url = [self normalizeUrl:input];
    TabItem *tab = [self activeTab];
    if (!tab) return;
    tab.url = url;
    tab.favicon = GetDefaultFaviconType(url);
    tab.faviconUrl = GetFaviconUrl(url);
    tab.isHibernated = NO;
    tab.scrollX = 0;
    tab.scrollY = 0;
    tab.lastActive = [NSDate date];
    [tab recordNavigation:url];
    if ([url isEqualToString:@"eclipse://settings"]) {
        tab.title = @"Preferences";
        tab.isLoading = NO;
    } else if ([url hasPrefix:@"eclipse://"]) {
        tab.title = @"New Tab";
        tab.isLoading = NO;
    } else {
        tab.title = @"Loading...";
        tab.isLoading = YES;
    }

    NSString *wvTarget = url;
    if ([url isEqualToString:@"eclipse://newtab"]) wvTarget = @"eclipse://localhost/newtab";
    else if ([url isEqualToString:@"eclipse://settings"]) wvTarget = @"eclipse://localhost/settings";
    [tab.webview loadRequest:[NSURLRequest requestWithURL:[NSURL URLWithString:wvTarget]]];
    [self broadcastState];
}

- (void)goBackActive {
    TabItem *tab = [self activeTab];
    if (!tab) return;
    if (tab.historyIndex > 0) {
        tab.historyIndex--;
        NSString *target = tab.history[tab.historyIndex];
        tab.url = target;
        tab.favicon = GetDefaultFaviconType(target);
        tab.faviconUrl = GetFaviconUrl(target);
        NSString *wvTarget = target;
        if ([target isEqualToString:@"eclipse://newtab"]) wvTarget = @"eclipse://localhost/newtab";
        else if ([target isEqualToString:@"eclipse://settings"]) wvTarget = @"eclipse://localhost/settings";
        [tab.webview loadRequest:[NSURLRequest requestWithURL:[NSURL URLWithString:wvTarget]]];
    } else {
        [tab.webview evaluateJavaScript:@"history.back();" completionHandler:nil];
    }
    [self broadcastState];
}

- (void)goForwardActive {
    TabItem *tab = [self activeTab];
    if (!tab) return;
    if (tab.historyIndex + 1 < tab.history.count) {
        tab.historyIndex++;
        NSString *target = tab.history[tab.historyIndex];
        tab.url = target;
        tab.favicon = GetDefaultFaviconType(target);
        tab.faviconUrl = GetFaviconUrl(target);
        NSString *wvTarget = target;
        if ([target isEqualToString:@"eclipse://newtab"]) wvTarget = @"eclipse://localhost/newtab";
        else if ([target isEqualToString:@"eclipse://settings"]) wvTarget = @"eclipse://localhost/settings";
        [tab.webview loadRequest:[NSURLRequest requestWithURL:[NSURL URLWithString:wvTarget]]];
    } else {
        [tab.webview evaluateJavaScript:@"history.forward();" completionHandler:nil];
    }
    [self broadcastState];
}

- (void)focusOmnibar {
    if ([self.config[@"sidebar_collapsed"] boolValue]) {
        self.config[@"sidebar_collapsed"] = @NO;
        [self saveConfig];
        [self updateLayout];
    }
    [self.window makeFirstResponder:self.sidebarWebView];
    [self.sidebarWebView evaluateJavaScript:@"window.focusOmnibar && window.focusOmnibar();" completionHandler:nil];
}

- (void)openSettingsDrawer {
    if ([self.config[@"sidebar_collapsed"] boolValue]) {
        self.config[@"sidebar_collapsed"] = @NO;
        [self saveConfig];
        [self updateLayout];
    }
    [self.sidebarWebView evaluateJavaScript:@"window.openPreferencesDrawer && window.openPreferencesDrawer();" completionHandler:nil];
}

- (void)openSettingsTab {
    for (TabItem *t in self.tabs) {
        if ([t.url isEqualToString:@"eclipse://settings"] && [t.workspace isEqualToString:self.currentWorkspace]) {
            [self selectTabWithId:t.tabId];
            return;
        }
    }
    [self createTabWithUrl:@"eclipse://settings" select:YES];
}

- (void)syncConfigToTab:(TabItem *)tab {
    if (!tab.webview) return;
    NSData *data = [NSJSONSerialization dataWithJSONObject:self.config options:0 error:nil];
    if (!data) return;
    NSString *json = [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding];
    NSString *script = [NSString stringWithFormat:@"window.__ECLIPSE_CONFIG__ = %@; if (window.onEclipseConfigUpdated) window.onEclipseConfigUpdated(%@);", json, json];
    [tab.webview evaluateJavaScript:script completionHandler:nil];
}

- (void)updateLayout {
    NSRect sBounds = [self sidebarBounds];
    self.sidebarEffectView.frame = sBounds;
    self.sidebarWebView.frame = sBounds;

    NSRect cBounds = [self contentBounds];
    TabItem *active = [self activeTab];
    if (active.webview) {
        active.webview.frame = cBounds;
    }
    [self broadcastState];
}

- (void)userContentController:(WKUserContentController *)userContentController didReceiveScriptMessage:(WKScriptMessage *)message {
    if (![message.name isEqualToString:@"ipc"]) return;
    NSString *body = [message.body isKindOfClass:[NSString class]] ? message.body : [message.body description];
    NSData *data = [body dataUsingEncoding:NSUTF8StringEncoding];
    if (!data) return;
    NSDictionary *dict = [NSJSONSerialization JSONObjectWithData:data options:0 error:nil];
    if ([dict isKindOfClass:[NSDictionary class]]) {
        [self handleIpcDictionary:dict fromTab:nil];
    }
}

- (void)handleIpcDictionary:(NSDictionary *)cmd fromTab:(TabItem *)sourceTab {
    NSString *action = cmd[@"action"];
    if (![action isKindOfClass:[NSString class]]) return;

    if ([action isEqualToString:@"ready"]) {
        [self broadcastState];
    } else if ([action isEqualToString:@"navigate"]) {
        NSString *u = cmd[@"url"];
        if ([u isKindOfClass:[NSString class]]) [self navigateActive:u];
    } else if ([action isEqualToString:@"new_tab"]) {
        NSString *u = [cmd[@"url"] isKindOfClass:[NSString class]] ? cmd[@"url"] : @"eclipse://newtab";
        [self createTabWithUrl:u select:YES];
    } else if ([action isEqualToString:@"select_tab"]) {
        NSUInteger tid = [cmd[@"id"] unsignedIntegerValue];
        if (tid > 0) [self selectTabWithId:tid];
    } else if ([action isEqualToString:@"select_tab_index"]) {
        NSUInteger idx = [cmd[@"index"] unsignedIntegerValue];
        NSMutableArray<TabItem *> *visible = [NSMutableArray array];
        for (TabItem *t in self.tabs) {
            if (t.isPinned || [t.workspace isEqualToString:self.currentWorkspace]) {
                [visible addObject:t];
            }
        }
        if (idx < visible.count) {
            [self selectTabWithId:visible[idx].tabId];
        } else if (idx == 8 && visible.count > 0) {
            [self selectTabWithId:visible.lastObject.tabId];
        }
    } else if ([action isEqualToString:@"close_tab"]) {
        NSUInteger tid = [cmd[@"id"] respondsToSelector:@selector(unsignedIntegerValue)]
            ? [cmd[@"id"] unsignedIntegerValue]
            : (sourceTab ? sourceTab.tabId : self.activeTabId);
        [self closeTabWithId:tid];
    } else if ([action isEqualToString:@"go_back"]) {
        [self goBackActive];
    } else if ([action isEqualToString:@"go_forward"]) {
        [self goForwardActive];
    } else if ([action isEqualToString:@"reload"]) {
        [self menuReload:nil];
    } else if ([action isEqualToString:@"toggle_sidebar"]) {
        [self menuToggleSidebar:nil];
    } else if ([action isEqualToString:@"focus_omnibar"]) {
        [self focusOmnibar];
    } else if ([action isEqualToString:@"open_settings"]) {
        [self openSettingsDrawer];
    } else if ([action isEqualToString:@"open_settings_tab"]) {
        [self openSettingsTab];
    } else if ([action isEqualToString:@"toggle_pin"]) {
        NSUInteger tid = [cmd[@"id"] unsignedIntegerValue];
        for (TabItem *t in self.tabs) {
            if (t.tabId == tid) {
                t.isPinned = !t.isPinned;
                break;
            }
        }
        [self broadcastState];
    } else if ([action isEqualToString:@"set_workspace"]) {
        NSString *ws = cmd[@"workspace"];
        if ([ws isKindOfClass:[NSString class]]) [self switchWorkspace:ws];
    } else if ([action isEqualToString:@"update_config"]) {
        NSDictionary *newCfg = cmd[@"config"];
        if ([newCfg isKindOfClass:[NSDictionary class]]) {
            NSArray *existingFavs = self.config[@"favorites"];
            [self.config addEntriesFromDictionary:newCfg];
            if (![self.config[@"favorites"] isKindOfClass:[NSArray class]] || [(NSArray *)self.config[@"favorites"] count] == 0) {
                if (existingFavs) self.config[@"favorites"] = [existingFavs mutableCopy];
            }
            [self saveConfig];
            NSString *cssScript = [self buildCustomCssScript:self.config[@"custom_css"]];
            for (TabItem *t in self.tabs) {
                if (t.webview) {
                    [t.webview evaluateJavaScript:cssScript completionHandler:nil];
                    [self syncConfigToTab:t];
                }
            }
            [self updateLayout];
        }
    } else if ([action isEqualToString:@"save_favorite"]) {
        NSString *title = [cmd[@"title"] isKindOfClass:[NSString class]] ? cmd[@"title"] : @"";
        NSString *url = [cmd[@"url"] isKindOfClass:[NSString class]] ? cmd[@"url"] : @"";
        if (url.length > 0) {
            NSString *normalized = [self normalizeUrl:url];
            NSString *icon = GetDefaultFaviconType(normalized);
            NSDictionary *entry = @{
                @"title": (title.length > 0 ? title : normalized),
                @"url": normalized,
                @"icon": icon
            };
            NSMutableArray *favs = [self.config[@"favorites"] mutableCopy] ?: [NSMutableArray array];
            id idxObj = cmd[@"index"];
            if ([idxObj respondsToSelector:@selector(unsignedIntegerValue)] && idxObj != [NSNull null]) {
                NSUInteger idx = [idxObj unsignedIntegerValue];
                if (idx < favs.count) {
                    favs[idx] = entry;
                } else {
                    [favs addObject:entry];
                }
            } else {
                [favs addObject:entry];
            }
            self.config[@"favorites"] = favs;
            [self saveConfig];
            for (TabItem *t in self.tabs) [self syncConfigToTab:t];
            [self broadcastState];
        }
    } else if ([action isEqualToString:@"delete_favorite"]) {
        id idxObj = cmd[@"index"];
        if ([idxObj respondsToSelector:@selector(unsignedIntegerValue)]) {
            NSUInteger idx = [idxObj unsignedIntegerValue];
            NSMutableArray *favs = [self.config[@"favorites"] mutableCopy];
            if (idx < favs.count) {
                [favs removeObjectAtIndex:idx];
                self.config[@"favorites"] = favs;
                [self saveConfig];
                for (TabItem *t in self.tabs) [self syncConfigToTab:t];
                [self broadcastState];
            }
        }
    } else if ([action isEqualToString:@"update_scroll"]) {
        TabItem *t = sourceTab ?: [self activeTab];
        if (t) {
            t.scrollX = [cmd[@"x"] doubleValue];
            t.scrollY = [cmd[@"y"] doubleValue];
        }
    } else if ([action isEqualToString:@"update_favicon"]) {
        TabItem *t = sourceTab ?: [self activeTab];
        NSString *fav = cmd[@"favicon"];
        if (t && [fav isKindOfClass:[NSString class]] && ![t.faviconUrl isEqualToString:fav]) {
            t.faviconUrl = fav;
            [self broadcastState];
        }
    } else if ([action isEqualToString:@"window_close"]) {
        [NSApp terminate:nil];
    } else if ([action isEqualToString:@"window_minimize"]) {
        [self.window miniaturize:nil];
    } else if ([action isEqualToString:@"window_zoom"]) {
        [self.window toggleFullScreen:nil];
    } else if ([action isEqualToString:@"drag_window"]) {
        NSEvent *ev = [NSApp currentEvent];
        if (ev) [self.window performWindowDragWithEvent:ev];
    }
}

- (void)handlePageStarted:(TabItem *)tab url:(NSString *)urlStr {
    NSString *canonical = CanonicalizeEclipseUrl(urlStr);
    tab.url = canonical;
    tab.isLoading = ![canonical hasPrefix:@"eclipse://"];
    tab.favicon = GetDefaultFaviconType(canonical);
    tab.faviconUrl = GetFaviconUrl(canonical);
    [tab recordNavigation:canonical];
    [self broadcastState];
}

- (void)handlePageFinished:(TabItem *)tab url:(NSString *)urlStr {
    NSString *canonical = CanonicalizeEclipseUrl(urlStr);
    tab.url = canonical;
    tab.isLoading = NO;
    tab.favicon = GetDefaultFaviconType(canonical);
    if (!tab.faviconUrl) tab.faviconUrl = GetFaviconUrl(canonical);
    if (tab.hasPendingScrollRestore && tab.webview) {
        tab.hasPendingScrollRestore = NO;
        NSString *js = [NSString stringWithFormat:@"window.scrollTo(%.1f, %.1f); setTimeout(()=>window.scrollTo(%.1f, %.1f), 120);", tab.scrollX, tab.scrollY, tab.scrollX, tab.scrollY];
        [tab.webview evaluateJavaScript:js completionHandler:nil];
    }
    [self syncConfigToTab:tab];
    [self broadcastState];
}

- (void)handleTitleChanged:(TabItem *)tab title:(NSString *)title {
    if (title.length > 0 && ![tab.title isEqualToString:title]) {
        tab.title = title;
        [self broadcastState];
    }
}

- (void)handleMaintenanceTick {
    BOOL hibernatedAny = NO;
    NSInteger mins = [self.config[@"tab_hibernation_mins"] integerValue];
    if (mins > 0) {
        NSTimeInterval threshold = mins * 60.0;
        NSDate *now = [NSDate date];
        for (TabItem *t in self.tabs) {
            if (t.tabId != self.activeTabId && !t.isHibernated && t.webview) {
                if ([now timeIntervalSinceDate:t.lastActive] > threshold) {
                    [t.webview removeFromSuperview];
                    t.webview = nil;
                    t.isHibernated = YES;
                    t.isLoading = NO;
                    hibernatedAny = YES;
                }
            }
        }
    }
    if (hibernatedAny) {
        [self broadcastState];
    } else {
        double mb = GetProcessMemoryMB();
        if (fabs(mb - self.lastBroadcastMemoryMB) >= 1.0) {
            self.lastBroadcastMemoryMB = mb;
            NSString *js = [NSString stringWithFormat:@"window.updateMemoryUsage && window.updateMemoryUsage(%.1f);", mb];
            [self.sidebarWebView evaluateJavaScript:js completionHandler:nil];
        }
    }
}

- (void)broadcastState {
    TabItem *active = [self activeTab];
    NSMutableArray *tabStates = [NSMutableArray arrayWithCapacity:self.tabs.count];
    for (TabItem *t in self.tabs) {
        [tabStates addObject:@{
            @"id": @(t.tabId),
            @"title": t.title ?: @"New Tab",
            @"url": t.url ?: @"eclipse://newtab",
            @"favicon": t.favicon ?: @"globe",
            @"favicon_url": t.faviconUrl ?: [NSNull null],
            @"is_loading": @(t.isLoading),
            @"is_pinned": @(t.isPinned),
            @"is_hibernated": @(t.isHibernated),
            @"workspace": t.workspace ?: @"Default"
        }];
    }
    double mb = GetProcessMemoryMB();
    self.lastBroadcastMemoryMB = mb;

    NSDictionary *state = @{
        @"tabs": tabStates,
        @"active_tab_id": @(self.activeTabId),
        @"active_url": active.url ?: @"eclipse://newtab",
        @"active_title": active.title ?: @"New Tab",
        @"can_go_back": @(active ? (active.historyIndex > 0) : NO),
        @"can_go_forward": @(active ? (active.historyIndex + 1 < active.history.count) : NO),
        @"is_loading": @(active ? active.isLoading : NO),
        @"sidebar_collapsed": @([self.config[@"sidebar_collapsed"] boolValue]),
        @"current_workspace": self.currentWorkspace ?: @"Default",
        @"memory_mb": @(mb),
        @"config": self.config ?: @{},
        @"favorites": self.config[@"favorites"] ?: @[]
    };

    NSData *jsonData = [NSJSONSerialization dataWithJSONObject:state options:0 error:nil];
    if (jsonData) {
        NSString *json = [[NSString alloc] initWithData:jsonData encoding:NSUTF8StringEncoding];
        NSString *js = [NSString stringWithFormat:@"window.updateBrowserState && window.updateBrowserState(%@);", json];
        [self.sidebarWebView evaluateJavaScript:js completionHandler:nil];
    }
}

@end

int main(int argc, const char * argv[]) {
    @autoreleasepool {
        NSApplication *app = [NSApplication sharedApplication];
        [app setActivationPolicy:NSApplicationActivationPolicyRegular];
        BrowserController *delegate = [[BrowserController alloc] init];
        app.delegate = delegate;
        [app run];
    }
    return 0;
}
