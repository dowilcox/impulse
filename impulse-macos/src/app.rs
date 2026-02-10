// ---------------------------------------------------------------------------
// macOS Application Lifecycle
// ---------------------------------------------------------------------------
//
// This module will contain the NSApplication setup and AppDelegate:
//
// - applicationDidFinishLaunching: Create main window, load settings
// - applicationWillTerminate: Save settings, close LSP servers
// - applicationShouldTerminateAfterLastWindowClosed: return true
//
// Dependencies (when building on macOS):
//   objc2, objc2-foundation, objc2-app-kit
//
// Example setup:
//
//   use objc2_app_kit::NSApplication;
//   use objc2_foundation::MainThreadMarker;
//
//   let mtm = MainThreadMarker::new().unwrap();
//   let app = NSApplication::sharedApplication(mtm);
//   app.setActivationPolicy(NSApplicationActivationPolicyRegular);
//   // Set delegate, create menus, run
//   app.run();
