use super::*;
impl Shell {
    pub(super) fn toggle_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.terminal.take().is_some() {
            self.current()
                .view
                .update(cx, |view, cx| view.focus_input(window, cx));
            cx.notify();
            return;
        }
        if self.terminal_picker.is_some() {
            return;
        }
        let directory = self
            .current()
            .project
            .and_then(|id| self.groups.iter().find(|group| group.id == id))
            .and_then(|group| group.directory.clone());
        if let Some(directory) = directory {
            self.terminal =
                Some(cx.new(|cx| crate::terminal::TerminalHost::new(directory, window, cx)));
            cx.notify();
            return;
        }
        // Until the native workspace resolver is wired, require an explicit cwd
        // for projectless chats instead of guessing a directory or creating one.
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("选择终端工作目录".into()),
        });
        self.terminal_picker = Some(cx.spawn_in(window, async move |owner, cx| {
            let result = paths.await;
            let _ = owner.update_in(cx, |this, window, cx| {
                this.terminal_picker = None;
                if let Ok(Ok(Some(paths))) = result {
                    if let Some(directory) = paths.into_iter().next() {
                        this.terminal =
                            Some(cx.new(|cx| {
                                crate::terminal::TerminalHost::new(directory, window, cx)
                            }));
                    }
                }
                cx.notify();
            });
        }));
    }
}
