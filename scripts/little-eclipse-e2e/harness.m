// End-to-end harness for Little Eclipse windows (src/menu.m).
//
// It links menu.m as the app does and stands in for wry: a WKWebView
// subclass and an IPC handler class named the way objc2 names wry's classes,
// an eclipse:// scheme handler that (like wry's) may only ever be called for
// that subclass, and a UI delegate that calls eclipse_open_little_window the
// way the Rust new-window handler does. Clicks are real AppKit mouse events
// sent to the window, so WebKit sees a user gesture.
//
// Usage: harness <scenario>   (run.sh builds and runs it; it prints MSG,
// TAB-FALLBACK, RESULT and WINDOW lines and exits after a few seconds)
#import <Cocoa/Cocoa.h>
#import <WebKit/WebKit.h>
#import <objc/runtime.h>

extern void eclipse_register_tab_view(void *viewPtr, uint64_t tabId);
extern void *eclipse_open_little_window(void *configurationPtr, void *openerPtr, double width, double height,
                                        int hasPosition, double x, double y);
extern int eclipse_script_message_from_little_window(void);

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

@interface HarnessUIDelegate : NSObject <WKUIDelegate>
@end

@implementation HarnessUIDelegate
- (WKWebView *)webView:(WKWebView *)webView
    createWebViewWithConfiguration:(WKWebViewConfiguration *)configuration
               forNavigationAction:(WKNavigationAction *)action
                    windowFeatures:(WKWindowFeatures *)features {
    void *raw = NULL;
    if (features.width != nil && features.height != nil) {
        raw = eclipse_open_little_window((__bridge void *)configuration, (__bridge void *)webView,
                                         features.width.doubleValue, features.height.doubleValue, 0, 0, 0);
    }
    if (raw == NULL) {
        gTabFallbacks++;
        printf("TAB-FALLBACK %s\n", action.request.URL.absoluteString.UTF8String);
        return nil;
    }
    return (__bridge_transfer WKWebView *)raw;
}
@end

static NSArray<NSWindow *> *little_windows(void) {
    NSMutableArray<NSWindow *> *out = [NSMutableArray array];
    for (NSWindow *window in NSApp.windows) {
        if (window != gTabWindow && window.isVisible && [window.contentView isKindOfClass:[WKWebView class]]) {
            [out addObject:window];
        }
    }
    return out;
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

int main(int argc, char **argv) {
    @autoreleasepool {
        NSString *scenario = argc > 1 ? @(argv[1]) : @"legit";
        const char *portEnv = getenv("HARNESS_PORT");
        int port = portEnv != NULL ? atoi(portEnv) : 8831;
        [NSApplication sharedApplication];
        NSApp.activationPolicy = NSApplicationActivationPolicyAccessory;

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
        gTab.UIDelegate = uiDelegate;
        gTabWindow.contentView = gTab;
        [gTabWindow makeKeyAndOrderFront:nil];
        eclipse_register_tab_view((__bridge void *)gTab, 1);

        NSString *url = [NSString stringWithFormat:@"http://127.0.0.1:%d/op.html?s=%@", port, scenario];
        [gTab loadRequest:[NSURLRequest requestWithURL:[NSURL URLWithString:url]]];

        after(2.0, ^{
            [gTab evaluateJavaScript:@"window.webkit.messageHandlers.ipc.postMessage('tab:self')"
                   completionHandler:nil];
            click(gTabWindow, gTab);
        });
        if ([scenario isEqualToString:@"nested"]) {
            after(4.0, ^{
                for (NSWindow *window in little_windows()) {
                    WKWebView *webView = (WKWebView *)window.contentView;
                    if ([webView.URL.path isEqualToString:@"/nest.html"]) click(window, webView);
                }
            });
        }
        after(6.0, ^{
            NSArray<NSWindow *> *windows = little_windows();
            printf("RESULT scenario=%s little=%lu fallbacks=%d unsafe_scheme=%d\n", scenario.UTF8String,
                   (unsigned long)windows.count, gTabFallbacks, gUnsafeSchemeCalls);
            for (NSWindow *window in windows) {
                WKWebView *webView = (WKWebView *)window.contentView;
                printf("WINDOW title=\"%s\" subtitle=\"%s\" url=%s\n", window.title.UTF8String,
                       window.subtitle.UTF8String, webView.URL.absoluteString.UTF8String ?: "(nil)");
            }
            fflush(stdout);
            exit(0);
        });
        [NSApp run];
    }
    return 0;
}
