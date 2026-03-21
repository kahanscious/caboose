use super::*;

impl App {
    pub(super) fn should_insert_text(modifiers: KeyModifiers) -> bool {
        modifiers.is_empty() || modifiers == KeyModifiers::SHIFT
    }

    pub(super) fn handle_paste(&mut self, text: &str) {
        const PASTE_THRESHOLD_LINES: usize = 20;
        const PASTE_THRESHOLD_CHARS: usize = 2000;

        match self.state.dialog_stack.top_mut() {
            Some(DialogKind::ApiKeyInput(state)) => {
                // Strip newlines — API keys are single-line
                let clean: String = text.chars().filter(|c| *c != '\n' && *c != '\r').collect();
                state.input.push_str(&clean);
            }
            Some(DialogKind::McpServerInput(state)) => {
                let clean: String = text.chars().filter(|c| *c != '\n' && *c != '\r').collect();
                state.focused_input_mut().push_str(&clean);
            }
            Some(_) => {
                // Other overlays don't accept paste
            }
            None => {
                // Check if paste contains image file paths (drag-and-drop or pasted paths)
                let (image_paths, remainder) = crate::attachment::try_attach_pasted_images(text);

                for path in &image_paths {
                    match crate::attachment::read_image_attachment(path, &self.images_config()) {
                        Ok(att) => {
                            if let Some(ref info) = att.compression {
                                let msg = format!(
                                    "Compressed {}: {} → {}",
                                    att.display_name,
                                    crate::attachment::format_size(info.original_size),
                                    crate::attachment::format_size(info.compressed_size),
                                );
                                self.state
                                    .chat_messages
                                    .push(ChatMessage::System { content: msg });
                            }
                            self.state.attachments.push(att);
                        }
                        Err(e) => {
                            self.state.chat_messages.push(ChatMessage::Error {
                                content: format!("Failed to attach: {e}"),
                            });
                        }
                    }
                }

                // If everything was image paths, we're done
                if remainder.is_empty() && !image_paths.is_empty() {
                    return;
                }

                // Use remainder (non-image lines) as the paste text
                let effective_text: &str = if image_paths.is_empty() {
                    text
                } else {
                    remainder.as_str()
                };

                // Base screen (Home or Chat) — paste into input with threshold check
                let line_count = effective_text.lines().count();
                let char_count = effective_text.len();
                if line_count > PASTE_THRESHOLD_LINES || char_count > PASTE_THRESHOLD_CHARS {
                    self.state.dialog_stack.push(DialogKind::PasteConfirm {
                        text: effective_text.to_string(),
                        line_count,
                        char_count,
                    });
                } else {
                    self.state.input.push_str(effective_text);
                    self.record_text_input_activity(effective_text.len().max(16));
                }
            }
        }
    }

    pub(super) fn record_text_input_activity(&mut self, inserted_len: usize) {
        const RAPID_INPUT_GAP_MS: u64 = 50;
        const PASTE_LIKE_THRESHOLD: usize = 24;
        const PASTE_LIKE_GRACE_MS: u64 = 900;

        let now = Instant::now();
        let within_burst = self
            .state
            .last_text_input_at
            .is_some_and(|t| now.duration_since(t) <= Duration::from_millis(RAPID_INPUT_GAP_MS));
        if within_burst {
            self.state.rapid_input_streak += inserted_len.max(1);
        } else {
            self.state.rapid_input_streak = inserted_len.max(1);
        }
        self.state.last_text_input_at = Some(now);
        if self.state.rapid_input_streak >= PASTE_LIKE_THRESHOLD {
            self.state.paste_like_mode_until =
                Some(now + Duration::from_millis(PASTE_LIKE_GRACE_MS));
        }
    }

    pub(super) fn reset_text_input_activity(&mut self) {
        self.state.last_text_input_at = None;
        self.state.rapid_input_streak = 0;
        self.state.paste_like_mode_until = None;
    }

    pub(super) fn should_treat_enter_as_paste_newline(&self) -> bool {
        const PASTE_LIKE_RECENT_MS: u64 = 250;
        const PASTE_LIKE_ENTER_THRESHOLD: usize = 12;

        // Slash commands are never pasted — always let Enter execute them
        if self.state.input.content().trim_start().starts_with('/') {
            return false;
        }

        if self
            .state
            .paste_like_mode_until
            .is_some_and(|until| Instant::now() <= until)
        {
            return true;
        }

        self.state.last_text_input_at.is_some_and(|t| {
            Instant::now().duration_since(t) <= Duration::from_millis(PASTE_LIKE_RECENT_MS)
        }) && self.state.rapid_input_streak >= PASTE_LIKE_ENTER_THRESHOLD
    }
}
