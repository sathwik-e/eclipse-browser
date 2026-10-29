// End-to-end harness for Little Eclipse windows (src/menu.m).
//
// It links menu.m as the app does and stands in for wry: a WKWebView
// subclass and an IPC handler class named the way objc2 names wry's classes,
// an eclipse:// scheme handler that (like wry's) may only ever be called for
// that subclass, and a UI delegate that calls eclipse_open_little_window the
// way the Rust new-window handler does. Clicks are real AppKit mouse events
// sent to the window, so WebKit sees a user gesture.
//
// It also installs menu.m's menu (the callback prints ACTION lines) and
// treats its tab window as the browser window, so shortcuts can be pressed
// while a popup or one of its dialogs is key, and a navigation delegate that
// prints the policy context menu.m publishes for eclipse:// navigations.
//
// Usage: harness <scenario>   (run.sh builds and runs it; it prints MSG,
// TAB-FALLBACK, ACTION, POLICY, KEY, RESULT and WINDOW lines and exits after
// a few seconds; it exits with status 2 if it cannot become the key app)
#import <Cocoa/Cocoa.h>
#import <WebKit/WebKit.h>
#import <objc/runtime.h>

extern void eclipse_register_tab_view(void *viewPtr, uint64_t tabId);
extern void *eclipse_open_little_window(void *configurationPtr, void *openerPtr, double width, double height,
                                        int hasPosition, double x, double y);
extern int eclipse_script_message_from_little_window(void);
extern int eclipse_popup_request_wants_popup(void);
extern int eclipse_navigation_policy_context(void);
extern int eclipse_little_window_count_for_view(void *viewPtr);
extern void eclipse_install_menu(void (*callback)(const char *action));
extern void eclipse_configure_window(void *windowPtr);
extern int eclipse_open_standalone_little_window(const char *url, const char *userAgent, int adblock,
                                                 int focusAddress);
extern int eclipse_little_window_load(uint64_t windowId, const char *url);

static Class gFakeWryClass;
static WKWebView *gTab;
static NSWindow *gTabWindow;
static int gUnsafeSchemeCalls = 0, gTabFallbacks = 0;

static void scheme_start(id self, SEL _cmd, WKWebView *webView, id<WKURLSchemeTask> task) {
    (void)self; (void)_cmd;
    if (![webView isKindOfClass:gFakeWryClass]) {
        // wry would treat this view as a WryWebView here.
        gUnsafeSchemeCalls++;
        printf("UNSAFE scheme start from %s for %s\n", object_getClassName(webView),
               task.request.URL.absoluteString.UTF8String);
    }
    NSURLResponse *response = [[NSURLResponse alloc] initWithURL:task.request.URL MIMEType:@"text/plain"
                                           expectedContentLength:1 textEncodingName:nil];
    [task didReceiveResponse:response];
    [task didReceiveData:[@"x" dataUsingEncoding:NSUTF8StringEncoding]];
    [task didFinish];
}

static void scheme_stop(id self, SEL _cmd, WKWebView *webView, id<WKURLSchemeTask> task) {
    (void)self; (void)_cmd; (void)task;
    if (![webView isKindOfClass:gFakeWryClass]) gUnsafeSchemeCalls++;
}

static void ipc_receive(id self, SEL _cmd, WKUserContentController *controller, WKScriptMessage *message) {
    (void)self; (void)_cmd; (void)controller;
    printf("MSG %s little=%d\n", [message.body description].UTF8String,
           eclipse_script_message_from_little_window());
}

/* Stands in for Rust's handling of the menu callback: an address typed in a
   Little Eclipse window is passed through unchanged (app.rs resolves it; this
   checks menu.m's own refusal of anything but http(s)), and ⌥⌘N opens a
   blank standalone window. Asynchronous, like the event loop. */
static void menu_action(const char *action) {
    printf("ACTION %s\n", action);
    NSString *command = @(action);
    dispatch_async(dispatch_get_main_queue(), ^{
        NSString *prefix = @"little_window_address:";
        if ([command hasPrefix:prefix]) {
            NSString *rest = [command substringFromIndex:prefix.length];
            NSRange colon = [rest rangeOfString:@":"];
            if (colon.location == NSNotFound) return;
            uint64_t windowId = strtoull([rest substringToIndex:colon.location].UTF8String, NULL, 10);
            NSString *text = [rest substringFromIndex:colon.location + 1];
            int ok = eclipse_little_window_load(windowId, text.UTF8String);
            printf("LOAD %s ok=%d\n", text.UTF8String, ok);
        } else if ([command isEqualToString:@"new_little_window"]) {
            int ok = eclipse_open_standalone_little_window("", "", 0, 1);
            printf("NEWLITTLE ok=%d\n", ok);
        }
    });
}

/* Stands in for wry's navigation delegate and the Rust policy for eclipse://
   URLs: a navigation another webview started is refused (tab.rs refuses
   eclipse:// for it unless it is the browser's own load or back/forward). */
@interface HarnessNavigationDelegate : NSObject <WKNavigationDelegate>
@end

@implementation HarnessNavigationDelegate
- (void)webView:(WKWebView *)webView
    decidePolicyForNavigationAction:(WKNavigationAction *)action
                    decisionHandler:(void (^)(WKNavigationActionPolicy))decisionHandler {
    (void)webView;
    NSURL *url = action.request.URL;
    if (![url.scheme isEqualToString:@"eclipse"]) {
        decisionHandler(WKNavigationActionPolicyAllow);
        return;
    }
    int context = eclipse_navigation_policy_context();
    BOOL allow = (context & 4) == 0;
    printf("POLICY %s ctx=%d allow=%d\n", url.absoluteString.UTF8String, context, allow);
    decisionHandler(allow ? WKNavigationActionPolicyAllow : WKNavigationActionPolicyCancel);
}
@end

@interface HarnessUIDelegate : NSObject <WKUIDelegate>
@end

@implementation HarnessUIDelegate
- (WKWebView *)webView:(WKWebView *)webView
    createWebViewWithConfiguration:(WKWebViewConfiguration *)configuration
               forNavigationAction:(WKNavigationAction *)action
                    windowFeatures:(WKWindowFeatures *)features {
    // As tab.rs: a size, or anything else WebKit reads as a popup window.
    void *raw = NULL;
    BOOL sized = features.width != nil && features.height != nil;
    if (sized || eclipse_popup_request_wants_popup()) {
        raw = eclipse_open_little_window((__bridge void *)configuration, (__bridge void *)webView,
                                         sized ? features.width.doubleValue : 0,
                                         sized ? features.height.doubleValue : 0, 0, 0, 0);
    }
    if (raw == NULL) {
        gTabFallbacks++;
        printf("TAB-FALLBACK %s\n", action.request.URL.absoluteString.UTF8String);
        return nil;
    }
    return (__bridge_transfer WKWebView *)raw;
}
@end

static NSView *find_view(NSView *root, BOOL (^match)(NSView *)) {
    if (root == nil) return nil;
    if (match(root)) return root;
    for (NSView *child in root.subviews) {
        NSView *found = find_view(child, match);
        if (found != nil) return found;
    }
    return nil;
}

static WKWebView *window_webview(NSWindow *window) {
    return (WKWebView *)find_view(window.contentView, ^BOOL(NSView *v) { return [v isKindOfClass:[WKWebView class]]; });
}

static NSView *window_control(NSWindow *window, NSString *identifier) {
    return find_view(window.contentView, ^BOOL(NSView *v) { return [v.identifier isEqualToString:identifier]; });
}

static NSArray<NSWindow *> *little_windows(void) {
    NSMutableArray<NSWindow *> *out = [NSMutableArray array];
    for (NSWindow *window in NSApp.windows) {
        if (window != gTabWindow && window.isVisible && !window.isSheet && window_webview(window) != nil) {
            [out addObject:window];
        }
    }
    return out;
}

/* The host label on a Little Eclipse window's top bar. */
static NSString *host_label(NSWindow *window) {
    NSButton *host = (NSButton *)window_control(window, @"eclipse-little-host");
    return host != nil ? host.title : @"(no label)";
}

static void print_editing(NSWindow *window, const char *when) {
    NSTextField *field = (NSTextField *)window_control(window, @"eclipse-little-address");
    NSView *host = window_control(window, @"eclipse-little-host");
    BOOL focused = field != nil && field.currentEditor != nil && window.firstResponder == field.currentEditor;
    printf("EDITING %s field=%d focused=%d host_hidden=%d text=\"%s\"\n", when, field != nil && !field.hidden,
           focused, host.hidden, field.stringValue.UTF8String ?: "");
}

/* Types `text` into the address field being edited, then sends `command`
   (insertNewline: is Return, cancelOperation: is Escape). */
static void type_in_address(NSWindow *window, NSString *text, SEL command) {
    NSTextField *field = (NSTextField *)window_control(window, @"eclipse-little-address");
    NSTextView *editor = (NSTextView *)field.currentEditor;
    if (editor == nil) {
        printf("NO EDITOR\n");
        return;
    }
    if (text != nil) editor.string = text;
    [editor doCommandBySelector:command];
}

static void click(NSWindow *window, NSView *view) {
    NSPoint point = [view convertPoint:NSMakePoint(NSMidX(view.bounds), NSMidY(view.bounds)) toView:nil];
    NSTimeInterval now = NSProcessInfo.processInfo.systemUptime;
    NSEvent *down = [NSEvent mouseEventWithType:NSEventTypeLeftMouseDown location:point modifierFlags:0
                                      timestamp:now windowNumber:window.windowNumber context:nil
                                    eventNumber:1 clickCount:1 pressure:1];
    NSEvent *up = [NSEvent mouseEventWithType:NSEventTypeLeftMouseUp location:point modifierFlags:0
                                    timestamp:now windowNumber:window.windowNumber context:nil
                                  eventNumber:2 clickCount:1 pressure:0];
    [window sendEvent:down];
    [window sendEvent:up];
}

static void after(double seconds, dispatch_block_t block) {
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(seconds * NSEC_PER_SEC)), dispatch_get_main_queue(), block);
}

/* A key press through the main menu's key equivalents, as AppKit sends it
   while the key window (a popup, or a dialog sheet on one) does not take it. */
static void press(NSString *key, NSEventModifierFlags modifiers) {
    NSEvent *event = [NSEvent keyEventWithType:NSEventTypeKeyDown location:NSZeroPoint modifierFlags:modifiers
                                     timestamp:NSProcessInfo.processInfo.systemUptime
                                  windowNumber:NSApp.keyWindow.windowNumber context:nil characters:key
                   charactersIgnoringModifiers:key isARepeat:NO keyCode:0];
    BOOL handled = [NSApp.mainMenu performKeyEquivalent:event];
    printf("PRESS cmd-%s handled=%d\n", key.UTF8String, handled);
}

static const char *key_window_name(void) {
    NSWindow *key = NSApp.keyWindow;
    if (key == nil) return "none";
    if (key == gTabWindow) return "tab";
    if (key.isSheet) return "sheet";
    return [little_windows() containsObject:key] ? "little" : "other";
}

static void print_key(void) {
    printf("KEY %s\n", key_window_name());
}

/* Runs `block` once `window` is key (synthetic clicks reach WebKit only in
   the active app's key window); exits with status 2 after `timeout`. */
static void when_key(NSWindow *window, double timeout, dispatch_block_t block) {
    if (window.isKeyWindow) {
        block();
        return;
    }
    if (timeout <= 0) {
        printf("NOT KEY: the harness could not become the active app, cannot click\n");
        fflush(stdout);
        exit(2);
    }
    [NSApp activateIgnoringOtherApps:YES];
    [window makeKeyAndOrderFront:nil];
    after(0.1, ^{ when_key(window, timeout - 0.1, block); });
}

int main(int argc, char **argv) {
    @autoreleasepool {
        NSString *scenario = argc > 1 ? @(argv[1]) : @"legit";
        const char *portEnv = getenv("HARNESS_PORT");
        int port = portEnv != NULL ? atoi(portEnv) : 8831;
        [NSApplication sharedApplication];
        NSApp.activationPolicy = NSApplicationActivationPolicyAccessory;
        eclipse_install_menu(menu_action);

        // objc2 names classes "<module path>::<type><crate version>".
        gFakeWryClass = objc_allocateClassPair([WKWebView class],
                                               "harness::wkwebview::class::wry_web_view::WryWebView0.0.0", 0);
        objc_registerClassPair(gFakeWryClass);
        Class ipcClass = objc_allocateClassPair(
            [NSObject class], "harness::wkwebview::class::wry_web_view_delegate::WryWebViewDelegate0.0.0", 0);
        class_addProtocol(ipcClass, @protocol(WKScriptMessageHandler));
        class_addMethod(ipcClass, @selector(userContentController:didReceiveScriptMessage:), (IMP)ipc_receive,
                        "v@:@@");
        objc_registerClassPair(ipcClass);
        Class schemeClass = objc_allocateClassPair([NSObject class], "harnessURLSchemeHandler", 0);
        class_addProtocol(schemeClass, @protocol(WKURLSchemeHandler));
        class_addMethod(schemeClass, @selector(webView:startURLSchemeTask:), (IMP)scheme_start, "v@:@@");
        class_addMethod(schemeClass, @selector(webView:stopURLSchemeTask:), (IMP)scheme_stop, "v@:@@");
        objc_registerClassPair(schemeClass);

        WKWebViewConfiguration *configuration = [[WKWebViewConfiguration alloc] init];
        configuration.websiteDataStore = [WKWebsiteDataStore nonPersistentDataStore];
        [configuration setURLSchemeHandler:[[schemeClass alloc] init] forURLScheme:@"eclipse"];
        [configuration.userContentController addScriptMessageHandler:[[ipcClass alloc] init] name:@"ipc"];

        gTabWindow = [[NSWindow alloc] initWithContentRect:NSMakeRect(100, 100, 800, 600)
                                                 styleMask:NSWindowStyleMaskTitled
                                                   backing:NSBackingStoreBuffered
                                                     defer:NO];
        gTab = [[gFakeWryClass alloc] initWithFrame:NSMakeRect(0, 0, 800, 600) configuration:configuration];
        HarnessUIDelegate *uiDelegate = [[HarnessUIDelegate alloc] init];
        HarnessNavigationDelegate *navigationDelegate = [[HarnessNavigationDelegate alloc] init];
        gTab.UIDelegate = uiDelegate;
        gTab.navigationDelegate = navigationDelegate;
        gTabWindow.contentView = gTab;
        eclipse_configure_window((__bridge void *)gTabWindow);
        [NSApp activateIgnoringOtherApps:YES];
        [gTabWindow makeKeyAndOrderFront:nil];
        eclipse_register_tab_view((__bridge void *)gTab, 1);

        NSString *url = [NSString stringWithFormat:@"http://127.0.0.1:%d/op.html?s=%@", port, scenario];
        [gTab loadRequest:[NSURLRequest requestWithURL:[NSURL URLWithString:url]]];

        WKWebView *(^littleWebView)(NSString *) = ^WKWebView *(NSString *path) {
            for (NSWindow *window in little_windows()) {
                WKWebView *webView = window_webview(window);
                if ([webView.URL.path isEqualToString:path]) return webView;
            }
            return nil;
        };
        // Everything below runs relative to the first click on the tab.
        void (^steps)(void) = ^{
            [gTab evaluateJavaScript:@"window.webkit.messageHandlers.ipc.postMessage('tab:self')"
                   completionHandler:nil];
            click(gTabWindow, gTab);
            if ([scenario isEqualToString:@"nested"]) {
                after(2.0, ^{
                    WKWebView *webView = littleWebView(@"/nest.html");
                    if (webView != nil) click(webView.window, webView);
                });
            } else if ([scenario isEqualToString:@"openernav"]) {
                // The tab's own page, then the browser, open internal pages.
                after(2.0, ^{
                    [gTab evaluateJavaScript:@"location = 'eclipse://localhost/newtab'" completionHandler:nil];
                });
                after(2.8, ^{
                    [gTab loadRequest:[NSURLRequest requestWithURL:[NSURL URLWithString:@"eclipse://localhost/notes"]]];
                });
            } else if ([scenario isEqualToString:@"print2"]) {
                // print(); print() in the popup, then one ⌘W.
                after(2.0, ^{ press(@"w", NSEventModifierFlagCommand); });
            } else if ([scenario isEqualToString:@"printclose"]) {
                // print(); print(), then window.close(): nothing may keep the
                // closed popup (and its page) alive.
                after(3.8, ^{
                    for (NSWindow *w in NSApp.windows) {
                        if (w != gTabWindow && !w.isSheet && window_webview(w) != nil) {
                            printf("LEFT visible=%d sheet=%d\n", w.isVisible, w.attachedSheet != nil);
                        }
                    }
                });
            } else if ([scenario isEqualToString:@"sheet"]) {
                after(2.0, ^{
                    print_key();
                    press(@"r", NSEventModifierFlagCommand);
                });
                after(2.3, ^{ press(@"w", NSEventModifierFlagCommand); });
            } else if ([scenario isEqualToString:@"refocus"] || [scenario isEqualToString:@"focus"]) {
                after(1.5, ^{ [gTabWindow makeKeyAndOrderFront:nil]; });
                after(2.0, ^{
                    print_key();
                    click(gTabWindow, gTab);
                });
                after(3.5, ^{ print_key(); });
            } else if ([scenario isEqualToString:@"menu"]) {
                after(1.5, ^{
                    print_key();
                    press(@"=", NSEventModifierFlagCommand);
                });
                after(1.8, ^{ press(@"d", NSEventModifierFlagCommand); });
                after(2.1, ^{
                    WKWebView *webView = littleWebView(@"/pop.html");
                    if (@available(macOS 11.0, *)) printf("ZOOM %.2f\n", webView.pageZoom);
                    press(@"t", NSEventModifierFlagCommand);
                });
                after(3.5, ^{ print_key(); });
            } else if ([scenario isEqualToString:@"openin"]) {
                // The bar's "Open in Eclipse" button (a real click, through
                // the transparent title bar).
                after(1.5, ^{
                    WKWebView *webView = littleWebView(@"/pop.html");
                    NSView *open = window_control(webView.window, @"eclipse-little-open");
                    printf("HOST \"%s\"\n", host_label(webView.window).UTF8String);
                    if (open != nil) click(webView.window, open);
                    else printf("NO OPEN BUTTON\n");
                });
                after(3.5, ^{ print_key(); });
            } else if ([scenario isEqualToString:@"openkey"]) {
                after(1.5, ^{
                    print_key();
                    press(@"o", NSEventModifierFlagCommand);
                });
                // With the browser window in front ⌘O is not for it.
                after(3.0, ^{
                    print_key();
                    press(@"o", NSEventModifierFlagCommand);
                });
            } else if ([scenario isEqualToString:@"standalone"]) {
                // As a link from another app arrives (app.rs): no opener.
                printf("STANDALONE ok=%d\n",
                       eclipse_open_standalone_little_window(
                           [NSString stringWithFormat:@"http://localhost:%d/pop.html", port + 1].UTF8String,
                           "EclipseHarness/1.0", 0, 0));
                printf("STANDALONE-ECLIPSE ok=%d\n",
                       eclipse_open_standalone_little_window("eclipse://localhost/settings", "", 0, 0));
                printf("STANDALONE-FILE ok=%d\n",
                       eclipse_open_standalone_little_window("file:///etc/passwd", "", 0, 0));
                after(1.5, ^{
                    print_key();
                    WKWebView *webView = littleWebView(@"/pop.html");
                    [webView evaluateJavaScript:@"String(window.opener) + ' ' + navigator.userAgent"
                              completionHandler:^(id result, NSError *error) {
                                  (void)error;
                                  printf("PAGE %s\n", [[result description] UTF8String] ?: "(error)");
                              }];
                });
                // The tab goes away: a standalone window has no opener to follow.
                after(2.0, ^{ [gTab removeFromSuperview]; });
            } else if ([scenario isEqualToString:@"address"]) {
                // ⌥⌘N: a blank standalone window, address field focused.
                press(@"n", NSEventModifierFlagCommand | NSEventModifierFlagOption);
                after(1.0, ^{
                    NSWindow *w = little_windows().firstObject;
                    print_key();
                    print_editing(w, "new");
                    type_in_address(w, [NSString stringWithFormat:@"http://localhost:%d/pop.html?typed", port + 1],
                                    @selector(insertNewline:));
                });
                after(2.5, ^{
                    NSWindow *w = little_windows().firstObject;
                    print_editing(w, "loaded");
                    NSView *host = window_control(w, @"eclipse-little-host");
                    if (host != nil) click(w, host);
                    print_editing(w, "clicked");
                    type_in_address(w, @"http://localhost:1/never", @selector(cancelOperation:));
                    print_editing(w, "escaped");
                });
            } else if ([scenario isEqualToString:@"addreclipse"]) {
                eclipse_open_standalone_little_window(
                    [NSString stringWithFormat:@"http://localhost:%d/pop.html", port + 1].UTF8String, "", 0, 0);
                after(1.5, ^{
                    NSWindow *w = little_windows().firstObject;
                    NSView *host = window_control(w, @"eclipse-little-host");
                    if (host != nil) click(w, host);
                    type_in_address(w, @"eclipse://localhost/settings", @selector(insertNewline:));
                });
                after(2.2, ^{
                    NSWindow *w = little_windows().firstObject;
                    [w makeFirstResponder:nil];
                    NSView *host = window_control(w, @"eclipse-little-host");
                    if (host != nil) click(w, host);
                    type_in_address(w, @"javascript:alert(1)", @selector(insertNewline:));
                });
            } else if ([scenario isEqualToString:@"linktab"]) {
                after(1.5, ^{
                    print_key();
                    WKWebView *webView = littleWebView(@"/lnk.html");
                    if (webView != nil) click(webView.window, webView);
                });
                after(3.5, ^{ print_key(); });
            }
            after(4.0, ^{
                NSArray<NSWindow *> *windows = little_windows();
                printf("RESULT scenario=%s little=%lu fallbacks=%d unsafe_scheme=%d count=%d\n",
                       scenario.UTF8String, (unsigned long)windows.count, gTabFallbacks, gUnsafeSchemeCalls,
                       eclipse_little_window_count_for_view((__bridge void *)gTab));
                printf("TABURL %s\n", gTab.URL.absoluteString.UTF8String ?: "(nil)");
                CGFloat top = NSMaxY([NSScreen screens].firstObject.frame);
                for (NSWindow *window in windows) {
                    WKWebView *webView = window_webview(window);
                    NSRect frame = window.frame;
                    printf("WINDOW title=\"%s\" host=\"%s\" url=%s frame=%.0f,%.0f,%.0fx%.0f\n",
                           window.title.UTF8String, host_label(window).UTF8String,
                           webView.URL.absoluteString.UTF8String ?: "(nil)", NSMinX(frame), top - NSMaxY(frame),
                           NSWidth(frame), NSHeight(frame));
                }
                fflush(stdout);
                exit(0);
            });
        };
        // Give the page time to load, then wait for the window to be key.
        after(1.5, ^{ when_key(gTabWindow, 5.0, steps); });
        [NSApp run];
    }
    return 0;
}
