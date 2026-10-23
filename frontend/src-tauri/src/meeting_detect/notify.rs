// meeting_detect/notify.rs
//
// The native "<App> meeting started" banner, one per call app.
//
// Why UNUserNotificationCenter: tauri-plugin-notification gives no click
// callback on desktop. The legacy NSUserNotificationCenter path (also what
// mac-notification-sys uses) is denied by usernoted on macOS 15 ("Denying
// message ... LegacyConnection"), so no banner ever shows there; the test Mac
// proved it. One delegate serves every banner and tells them apart by the
// request identifier, so two calls give two banners that never mix up clicks.

/// What the user did with a banner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Response {
    /// The Record button.
    Record,
    /// The banner body: bring the main window to the front.
    Open,
    /// The close button.
    Dismiss,
}

/// Banner identifier for an app key ("zoom" | "teams").
pub fn banner_id(app_key: &str) -> String {
    format!("{}{}", ID_PREFIX, app_key)
}

/// The app key a banner identifier names.
pub fn app_of_banner(id: &str) -> Option<&str> {
    id.strip_prefix(ID_PREFIX)
}

const ID_PREFIX: &str = "meeting-detect-";

/// Action identifiers: ours, and the two the system uses for a click on the
/// banner body and for the close button.
pub const RECORD_ACTION: &str = "meeting-detect-record";
pub const DEFAULT_ACTION: &str = "com.apple.UNNotificationDefaultActionIdentifier";
pub const DISMISS_ACTION: &str = "com.apple.UNNotificationDismissActionIdentifier";

pub fn response_for_action(action: &str) -> Option<Response> {
    match action {
        DEFAULT_ACTION => Some(Response::Open),
        RECORD_ACTION => Some(Response::Record),
        DISMISS_ACTION => Some(Response::Dismiss),
        _ => None,
    }
}

#[cfg(target_os = "macos")]
pub use mac::{init, show, withdraw};

#[cfg(not(target_os = "macos"))]
pub fn init(_on_response: Box<dyn Fn(&str, Response) + Send + Sync>) {}
#[cfg(not(target_os = "macos"))]
pub fn show(_app_key: &str, _title: &str, _body: &str) {}
#[cfg(not(target_os = "macos"))]
pub fn withdraw(_app_key: &str) {}

#[cfg(target_os = "macos")]
mod mac {
    use super::{app_of_banner, banner_id, response_for_action, Response, RECORD_ACTION};
    use block2::{DynBlock, RcBlock};
    use objc2::rc::Retained;
    use objc2::runtime::{Bool, ProtocolObject};
    use objc2::{define_class, msg_send, AnyThread, DefinedClass};
    use objc2_foundation::{NSArray, NSBundle, NSError, NSObject, NSObjectProtocol, NSSet, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotification,
        UNNotificationAction, UNNotificationActionOptions, UNNotificationCategory,
        UNNotificationCategoryOptions, UNNotificationPresentationOptions,
        UNNotificationRequest, UNNotificationResponse, UNUserNotificationCenter,
        UNUserNotificationCenterDelegate,
    };
    use std::sync::OnceLock;

    type Callback = Box<dyn Fn(&str, Response) + Send + Sync>;
    static CALLBACK: OnceLock<Callback> = OnceLock::new();
    /// The center holds its delegate weakly, so keep ours alive here.
    static DELEGATE: OnceLock<Retained<Delegate>> = OnceLock::new();

    const CATEGORY: &str = "meeting-detect";

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "MeetilyMeetingDetectDelegate"]
        struct Delegate;

        // SAFETY: Delegate is a plain NSObject subclass with no ivars, so the marker trait holds.
        unsafe impl NSObjectProtocol for Delegate {}

        // SAFETY: both methods below match the protocol selectors and signatures declared by objc2-user-notifications.
        unsafe impl UNUserNotificationCenterDelegate for Delegate {
            // Show the banner even while Meetily is the frontmost app.
            #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
            fn will_present(
                &self,
                _center: &UNUserNotificationCenter,
                _notification: &UNNotification,
                handler: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
            ) {
                handler.call((UNNotificationPresentationOptions::Banner
                    | UNNotificationPresentationOptions::List,));
            }

            #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
            fn did_receive(
                &self,
                _center: &UNUserNotificationCenter,
                response: &UNNotificationResponse,
                handler: &DynBlock<dyn Fn()>,
            ) {
                let action = response.actionIdentifier().to_string();
                let id = response.notification().request().identifier().to_string();
                if let (Some(app), Some(r)) = (app_of_banner(&id), response_for_action(&action)) {
                    if let Some(cb) = CALLBACK.get() {
                        cb(app, r);
                    }
                }
                handler.call(());
            }
        }
    );

    impl Delegate {
        fn new() -> Retained<Self> {
            let this = Self::alloc().set_ivars(());
            // SAFETY: `this` is freshly allocated with its ivars set, and NSObject init takes no arguments.
            unsafe { msg_send![super(this), init] }
        }
    }

    /// Installs the delegate and asks for permission; `on_response` gets (app key, response).
    pub fn init(on_response: Callback) {
        // The center aborts the process when the app has no bundle (tauri dev).
        if NSBundle::mainBundle().bundleIdentifier().is_none() {
            log::warn!("meeting_detect: not a bundled app, no native banners");
            return;
        }
        if CALLBACK.set(on_response).is_err() {
            return;
        }
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let delegate = Delegate::new();
        center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        let _ = DELEGATE.set(delegate);

        let record = UNNotificationAction::actionWithIdentifier_title_options(
            &NSString::from_str(RECORD_ACTION),
            &NSString::from_str("Record"),
            UNNotificationActionOptions::Foreground,
        );
        let category = UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(
            &NSString::from_str(CATEGORY),
            &NSArray::from_retained_slice(&[record]),
            &NSArray::new(),
            // The close button reports back, so it counts as Dismiss.
            UNNotificationCategoryOptions::CustomDismissAction,
        );
        center.setNotificationCategories(&NSSet::from_retained_slice(&[category]));

        // A grant already on file answers at once with no prompt.
        let done = RcBlock::new(|granted: Bool, error: *mut NSError| {
            if !granted.as_bool() || !error.is_null() {
                log::warn!("meeting_detect: notifications not allowed (granted={})", granted.as_bool());
            }
        });
        center.requestAuthorizationWithOptions_completionHandler(UNAuthorizationOptions::Alert, &done);
    }

    pub fn show(app_key: &str, title: &str, body: &str) {
        if DELEGATE.get().is_none() {
            return;
        }
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(title));
        content.setBody(&NSString::from_str(body));
        content.setCategoryIdentifier(&NSString::from_str(CATEGORY));
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
            &NSString::from_str(&banner_id(app_key)),
            &content,
            None,
        );
        center.addNotificationRequest_withCompletionHandler(&request, None);
    }

    /// Takes this app's banner off the screen and out of Notification Center.
    pub fn withdraw(app_key: &str) {
        if DELEGATE.get().is_none() {
            return;
        }
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let ids = NSArray::from_retained_slice(&[NSString::from_str(&banner_id(app_key))]);
        center.removeDeliveredNotificationsWithIdentifiers(&ids);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banner_ids_round_trip_per_app() {
        assert_eq!(app_of_banner(&banner_id("zoom")), Some("zoom"));
        assert_eq!(app_of_banner(&banner_id("teams")), Some("teams"));
        assert_ne!(banner_id("zoom"), banner_id("teams"));
        assert_eq!(app_of_banner("something-else"), None);
    }

    #[test]
    fn actions_map_to_responses() {
        assert_eq!(response_for_action(DEFAULT_ACTION), Some(Response::Open));
        assert_eq!(response_for_action(RECORD_ACTION), Some(Response::Record));
        assert_eq!(response_for_action(DISMISS_ACTION), Some(Response::Dismiss));
        assert_eq!(response_for_action("something-else"), None);
    }
}
