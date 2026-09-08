//! Native notifications belong to Kivio's bundle, not osascript / Script Editor.
//! Start native delivery on a bounded worker; Apple completes it asynchronously.

use block2::{DynBlock, RcBlock};
use objc2::{
    define_class, msg_send,
    rc::Retained,
    runtime::{Bool, ProtocolObject},
    ClassType,
};
use objc2_foundation::{NSBundle, NSError, NSObject, NSObjectProtocol, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification,
    UNNotificationPresentationOptions, UNNotificationRequest, UNUserNotificationCenter,
    UNUserNotificationCenterDelegate,
};
use std::{
    sync::{mpsc::SyncSender, OnceLock},
    time::{Duration, Instant},
};

define_class!(
    // SAFETY: NSObject has no subclassing requirements. This delegate has no
    // mutable state and its only callback immediately completes presentation.
    #[unsafe(super = NSObject)]
    struct KivioNotificationDelegate;

    unsafe impl NSObjectProtocol for KivioNotificationDelegate {}

    unsafe impl UNUserNotificationCenterDelegate for KivioNotificationDelegate {
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            completion: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            // Chat has already suppressed the conversation being viewed. Other
            // conversations and automations must still notify while Kivio is active.
            completion
                .call((UNNotificationPresentationOptions::Banner
                    | UNNotificationPresentationOptions::List,));
        }
    }
);

thread_local! {
    // UNUserNotificationCenter.delegate is weak. Retain it for the worker thread's
    // lifetime, otherwise foreground notifications silently disappear again.
    static DELEGATE: Retained<KivioNotificationDelegate> = unsafe {
        msg_send![KivioNotificationDelegate::class(), new]
    };
}

pub(super) fn show(app: &tauri::AppHandle, title: String, body: String) {
    static WORKER: OnceLock<Option<SyncSender<PendingNotification>>> = OnceLock::new();
    let worker = WORKER.get_or_init(|| {
        super::start_notification_worker(16, |message: PendingNotification| {
            if message.queued_at.elapsed() > Duration::from_secs(15) {
                eprintln!("macOS notification skipped: expired in delivery queue");
                return;
            }
            objc2::rc::autoreleasepool(|_| {
                show_native(&message.identifier, message.title, message.body);
            });
        })
        .map_err(|error| eprintln!("macOS notification worker failed to start: {error}"))
        .ok()
    });
    if let Some(worker) = worker {
        let message = PendingNotification {
            identifier: app.config().identifier.clone(),
            title,
            body,
            queued_at: Instant::now(),
        };
        // Never wait if usernoted is stuck or the notification queue is full.
        if worker.try_send(message).is_err() {
            eprintln!("macOS notification skipped: worker unavailable or queue full");
        }
    }
}

struct PendingNotification {
    identifier: String,
    title: String,
    body: String,
    queued_at: Instant,
}

fn show_native(identifier: &str, title: String, body: String) {
    // currentNotificationCenter throws an Objective-C exception for an unbundled
    // executable (cargo run / tauri dev). Check before calling it, not afterwards.
    let bundle = NSBundle::mainBundle();
    if !bundle.bundlePath().to_string().ends_with(".app")
        || bundle.bundleIdentifier().as_deref() != Some(&*NSString::from_str(identifier))
    {
        eprintln!("macOS notification skipped: launch the Kivio .app bundle to enable native notifications");
        return;
    }
    let center = UNUserNotificationCenter::currentNotificationCenter();
    DELEGATE.with(|delegate| center.setDelegate(Some(ProtocolObject::from_ref(&**delegate))));
    let permission = RcBlock::new(move |granted: Bool, error: *mut NSError| {
        // SAFETY: Apple's callback supplies a nullable NSError valid for this call.
        if let Some(error) = unsafe { error.as_ref() } {
            eprintln!("macOS notification authorization failed: {error}");
        } else if !granted.as_bool() {
            eprintln!(
                "macOS notifications denied: enable Kivio in System Settings > Notifications"
            );
        } else {
            let request = notification_request(&title, &body);
            let completion = RcBlock::new(|error: *mut NSError| {
                // SAFETY: Same callback lifetime guarantee as above.
                if let Some(error) = unsafe { error.as_ref() } {
                    eprintln!("macOS notification delivery failed: {error}");
                }
            });
            UNUserNotificationCenter::currentNotificationCenter()
                .addNotificationRequest_withCompletionHandler(&request, Some(&completion));
        }
    });
    // The OS prompts only on first use and respects an existing denial. No
    // semaphore, channel receive or process wait is involved in authorization.
    center.requestAuthorizationWithOptions_completionHandler(
        UNAuthorizationOptions::Alert,
        &permission,
    );
}

fn notification_request(title: &str, body: &str) -> Retained<UNNotificationRequest> {
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setBody(&NSString::from_str(body));
    UNNotificationRequest::requestWithIdentifier_content_trigger(
        &NSString::from_str(&uuid::Uuid::new_v4().to_string()),
        &content,
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_request_preserves_unicode_and_script_like_text_without_sending() {
        let title = "测试对话 🦀";
        let body = "回复 & <tag> \\\"quoted\\\" $(literal)";
        let first = notification_request(title, body);
        let second = notification_request(title, body);
        assert_eq!(first.content().title().to_string(), title);
        assert_eq!(first.content().body().to_string(), body);
        assert!(first.trigger().is_none());
        assert_ne!(first.identifier(), second.identifier());
    }
}
