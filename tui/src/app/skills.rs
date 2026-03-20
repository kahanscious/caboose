use super::*;

impl App {
    pub(super) fn handle_skill_creation_key(&mut self, key: crossterm::event::KeyCode) -> bool {
        let creation = match &self.state.skill_creation {
            Some(c) => c.clone(),
            None => return false,
        };

        let (content, companions) = match &creation.phase {
            crate::skills::creation::SkillCreationPhase::Preview {
                content,
                companion_files,
            } => (content.clone(), companion_files.clone()),
            _ => return false, // Only handle keys in preview phase
        };

        match key {
            crossterm::event::KeyCode::Char('p') => {
                self.save_created_skill(
                    &creation.name,
                    &content,
                    &companions,
                    crate::skills::creation::SkillScope::Project,
                );
                true
            }
            crossterm::event::KeyCode::Char('g') => {
                self.save_created_skill(
                    &creation.name,
                    &content,
                    &companions,
                    crate::skills::creation::SkillScope::Global,
                );
                true
            }
            crossterm::event::KeyCode::Char('e') => {
                // Edit — return to gathering with feedback prompt
                self.state.skill_creation.as_mut().unwrap().phase =
                    crate::skills::creation::SkillCreationPhase::Gathering;
                self.state.chat_messages.push(ChatMessage::System {
                    content: "Type your feedback to refine the skill:".into(),
                });
                true
            }
            crossterm::event::KeyCode::Char('c') => {
                // Cancel
                self.state.skill_creation = None;
                self.state.chat_messages.push(ChatMessage::System {
                    content: "Skill creation cancelled.".into(),
                });
                true
            }
            _ => false,
        }
    }

    /// Save a created skill to disk and reload.
    pub(super) fn save_created_skill(
        &mut self,
        name: &str,
        content: &str,
        companions: &[crate::skills::creation::CompanionFile],
        scope: crate::skills::creation::SkillScope,
    ) {
        // Check for existing skill at target
        if let Some(existing_path) = crate::skills::creation::skill_exists(name, scope) {
            self.state.chat_messages.push(ChatMessage::System {
                content: format!("Overwriting existing skill at {}", existing_path.display()),
            });
        }

        match crate::skills::creation::write_skill(name, content, companions, scope) {
            Ok(path) => {
                self.state.chat_messages.push(ChatMessage::System {
                    content: format!("Skill \"{name}\" saved to {}", path.display()),
                });

                // Reload skills
                let disabled = self
                    .state
                    .config
                    .skills
                    .as_ref()
                    .map(|s| s.disabled.clone())
                    .unwrap_or_default();
                self.state.skills =
                    crate::skills::loader::load_all_skills(std::path::Path::new("."), &disabled);

                self.state.chat_messages.push(ChatMessage::System {
                    content: format!(
                        "Skill list reloaded ({} skills available). Use /{name} to invoke it.",
                        self.state.skills.len()
                    ),
                });
            }
            Err(e) => {
                self.state.chat_messages.push(ChatMessage::Error {
                    content: format!("Failed to save skill: {e}"),
                });
            }
        }

        // Clear creation state
        self.state.skill_creation = None;
    }

    /// Toggle the currently selected skill's disabled state in the Skills picker.
    pub(super) fn toggle_skill_disabled(&mut self) {
        let Some(auto) = &self.state.slash_auto else {
            return;
        };
        let filtered = crate::tui::slash_auto::filter_skills(&self.state.skills, &auto.filter);
        let Some(&idx) = filtered.get(auto.selected) else {
            return;
        };
        let skill_name = self.state.skills[idx].name.clone();

        let skills_config = self
            .state
            .config
            .skills
            .get_or_insert_with(Default::default);
        let lower = skill_name.to_lowercase();
        if let Some(pos) = skills_config
            .disabled
            .iter()
            .position(|s| s.to_lowercase() == lower)
        {
            skills_config.disabled.remove(pos);
        } else {
            skills_config.disabled.push(skill_name);
        }

        // Persist to config file
        let project_config_exists = std::path::Path::new(".caboose/config.toml").exists();
        caboose_core::config::save_skills_disabled(&skills_config.disabled, project_config_exists);

        // Reload skills to reflect change
        let disabled = self
            .state
            .config
            .skills
            .as_ref()
            .map(|s| s.disabled.clone())
            .unwrap_or_default();
        self.state.skills =
            crate::skills::loader::load_all_skills(std::path::Path::new("."), &disabled);
    }

    /// Delete the currently selected user skill (not built-in) from disk.
    pub(super) fn delete_user_skill(&mut self) {
        let Some(auto) = &self.state.slash_auto else {
            return;
        };
        let filtered = crate::tui::slash_auto::filter_skills(&self.state.skills, &auto.filter);
        let Some(&idx) = filtered.get(auto.selected) else {
            return;
        };
        let skill = &self.state.skills[idx];

        // Only user skills (File source) can be deleted
        let path = match &skill.source {
            crate::skills::SkillSource::File(p) => p.clone(),
            crate::skills::SkillSource::Builtin => {
                self.state.chat_messages.push(ChatMessage::System {
                    content: "Cannot delete built-in skills.".into(),
                });
                return;
            }
        };

        let name = skill.name.clone();

        // Delete the file (or folder for folder-skills)
        if path.is_dir() {
            if std::fs::remove_dir_all(&path).is_err() {
                self.state.chat_messages.push(ChatMessage::Error {
                    content: format!("Failed to delete skill folder: {}", path.display()),
                });
                return;
            }
        } else if std::fs::remove_file(&path).is_err() {
            self.state.chat_messages.push(ChatMessage::Error {
                content: format!("Failed to delete skill file: {}", path.display()),
            });
            return;
        }

        self.state.chat_messages.push(ChatMessage::System {
            content: format!("Deleted skill \"{name}\""),
        });

        // Reload skills
        let disabled = self
            .state
            .config
            .skills
            .as_ref()
            .map(|s| s.disabled.clone())
            .unwrap_or_default();
        self.state.skills =
            crate::skills::loader::load_all_skills(std::path::Path::new("."), &disabled);

        // Clamp selection in picker
        if let Some(auto) = self.state.slash_auto.as_mut() {
            let count =
                crate::tui::slash_auto::filtered_skill_count(&self.state.skills, &auto.filter);
            if auto.selected >= count && count > 0 {
                auto.selected = count - 1;
            }
        }
    }

    /// Handle `/create-skill [name] [goal]` — start the skill creation flow.
    ///
    /// Supports both direct (`/create-skill deploy automate deploys`) and
    /// conversational (`/create-skill` → prompts for name → prompts for goal).
    pub(super) fn handle_create_skill_command(&mut self, args_str: &str) {
        // Always transition to chat screen
        if matches!(self.state.dialog_stack.base, Screen::Home) {
            self.state.dialog_stack.base = Screen::Chat;
            self.state.dialog_stack.clear();
        }

        self.state.chat_messages.push(ChatMessage::User {
            content: if args_str.is_empty() {
                "/create-skill".to_string()
            } else {
                format!("/create-skill {args_str}")
            },
            images: vec![],
        });
        self.state.user_scrolled_up = false;

        let parts: Vec<&str> = args_str.splitn(2, char::is_whitespace).collect();
        let name = parts
            .first()
            .filter(|n| !n.is_empty())
            .map(|n| n.trim().to_lowercase());
        let goal = parts
            .get(1)
            .map(|g| g.trim())
            .filter(|g| !g.is_empty())
            .map(String::from);

        match (name, goal) {
            // Both provided — validate and start immediately
            (Some(name), Some(goal)) => {
                if crate::skills::creation::is_reserved_name(&name) {
                    self.state.chat_messages.push(ChatMessage::Error {
                        content: format!(
                            "'{name}' is a reserved command name. Choose a different name."
                        ),
                    });
                    return;
                }
                self.start_skill_creation(name, goal);
            }
            // Name only — ask for goal
            (Some(name), None) => {
                if crate::skills::creation::is_reserved_name(&name) {
                    self.state.chat_messages.push(ChatMessage::Error {
                        content: format!(
                            "'{name}' is a reserved command name. Choose a different name."
                        ),
                    });
                    return;
                }
                self.state.skill_creation = Some(crate::skills::creation::SkillCreationState {
                    name,
                    goal: String::new(),
                    phase: crate::skills::creation::SkillCreationPhase::AwaitingGoal,
                    question_count: 0,
                });
                self.state.chat_messages.push(ChatMessage::System {
                    content: "What should this skill do? Describe the goal in a sentence or two."
                        .into(),
                });
            }
            // Nothing — ask for name
            _ => {
                self.state.skill_creation = Some(crate::skills::creation::SkillCreationState {
                    name: String::new(),
                    goal: String::new(),
                    phase: crate::skills::creation::SkillCreationPhase::AwaitingName,
                    question_count: 0,
                });
                self.state.chat_messages.push(ChatMessage::System {
                    content: "Let's create a skill! What do you want to name it?".into(),
                });
            }
        }
    }

    /// Start the LLM-guided skill creation after name and goal are known.
    pub(super) fn start_skill_creation(&mut self, name: String, goal: String) {
        if !self.require_provider() {
            return;
        }

        self.state.skill_creation = Some(crate::skills::creation::SkillCreationState {
            name: name.clone(),
            goal: goal.clone(),
            phase: crate::skills::creation::SkillCreationPhase::Gathering,
            question_count: 0,
        });

        self.state.chat_messages.push(ChatMessage::System {
            content: format!(
                "Creating skill \"{name}\" — the assistant will ask a few questions to refine it."
            ),
        });

        // Inject creation system prompt and send initial message
        let creation_prompt = crate::skills::creation::system_prompt(&name, &goal);
        let initial_msg = format!(
            "{creation_prompt}\n\nI want to create a skill called \"{name}\". Goal: {goal}"
        );

        let tool_defs = self.build_tool_defs();

        self.state.agent.send_message(
            initial_msg,
            self.provider.as_ref().unwrap().as_ref(),
            &tool_defs,
        );
    }
}
