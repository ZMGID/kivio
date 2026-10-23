use super::Preview;
use crate::providers::ProviderSettings;
use gpui_kit::{
    AppContext, ClipboardItem, Image, ImageFormat, SharedString, TestAppContext, WindowOptions,
    test::TestWindowExt,
};

// Uses native picker and clipboard actions. Corrupt input must leave the user's
// draft/staged path untouched; a later valid image must reach the real adapter.
#[gpui_kit::test]
fn picker_failure_and_clipboard_image_keep_the_correct_draft(cx: &mut TestAppContext) {
    let data = tempfile::tempdir().unwrap();
    let broken = data.path().join("broken.png");
    std::fs::write(&broken, b"not an image").unwrap();
    let (provider, server) = crate::chat_runtime::test_support::serve(
        vec![crate::chat_runtime::test_support::response(
            "image received",
        )],
        false,
    );
    std::fs::write(data.path().join("providers.json"), serde_json::to_vec(&serde_json::json!({"providers":[provider],"defaultModels":{"chat":{"providerId":"native-test","model":"test-model"}}})).unwrap()).unwrap();
    let (window, view) = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            let providers =
                cx.new(|cx| ProviderSettings::new(data.path().join("providers.json"), window, cx));
            cx.new(|cx| {
                let mut view = Preview::new(window, cx);
                view.configure(providers, None, "conv_image".into(), data.path().to_owned());
                view
            })
        })
        .unwrap()
    });
    cx.run_until_parked();
    cx.simulate_input(window, "看看附件");
    window
        .update(cx, |_, window, cx| {
            window.render_frame(cx);
            window.click("attachments", cx);
        })
        .unwrap();
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|options| {
        assert!(options.files && options.directories && options.multiple);
        Some(vec![broken.clone()])
    });
    cx.run_until_parked();
    cx.simulate_keystrokes(window, "enter");
    cx.run_until_parked();
    view.read_with(cx, |view, cx| {
        assert_eq!(view.input.read(cx).value(), "看看附件");
        assert_eq!(view.attachments, [broken.clone()]);
        assert!(
            view.attachment_error
                .as_ref()
                .is_some_and(|error| error.contains("解码"))
        );
        assert!(view.snapshot(cx).0.is_empty());
    });
    assert!(
        !data.path().join("conv_image_attachments").exists(),
        "failed media must clean its uncommitted copy"
    );
    window
        .update(cx, |_, window, cx| {
            window.render_frame(cx);
            window.click(
                SharedString::from(format!("attachment-{}", broken.display())),
                cx,
            );
        })
        .unwrap();
    cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(
        ImageFormat::Png,
        include_bytes!("../../test-fixtures/attachment.png").to_vec(),
    )));
    window
        .update(cx, |_, window, cx| {
            view.update(cx, |view, cx| view.focus_input(window, cx))
        })
        .unwrap();
    cx.simulate_keystrokes(window, "secondary-v");
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert_eq!(view.attachments.len(), 1);
        assert_eq!(view.thumbnails.len(), 1);
    });
    cx.simulate_keystrokes(window, "enter");
    cx.run_until_parked();
    let history = view.read_with(cx, |view, cx| {
        assert!(view.attachments.is_empty());
        assert!(view.input.read(cx).value().is_empty());
        let (history, _) = view.snapshot(cx);
        assert_eq!(history[0].attachments.len(), 1);
        history
    });
    let requests = server.join().unwrap();
    assert!(requests[0].to_string().contains("data:image/png;base64,"));
    let copied = &history[0].attachments[0].path;
    assert_eq!(
        std::fs::read(data.path().join("conv_image_attachments").join(copied)).unwrap(),
        include_bytes!("../../test-fixtures/attachment.png")
    );
    cx.run_until_parked();
}
