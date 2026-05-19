use anyhow::Result;
use eframe::CreationContext;
use egui::{Align, Color32, Key, Layout, RichText, ScrollArea, TextEdit, Ui};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::db::Db;
use crate::inference;
use crate::llm::{EnhanceTrigger, JobKind, JobOutput, JobResult, LlmConfig, LlmManager, Provider};
use crate::models::{Category, CommandNote, ExportBundle};
use crate::search::SearchEngine;
use crate::update::UpdateChecker;

#[allow(dead_code)]
const APP_NAME: &str = "📚 Spellbook";

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum View {
    #[default]
    All,
    Favorites,
    Recent,
    Category(i64),
    Trash,
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Prefs {
    view: View,
    selected_command_id: Option<i64>,
    search: String,
}

const PREFS_KEY: &str = "spellbook_prefs";

pub struct App {
    db: Db,
    engine: SearchEngine,

    // cached state
    commands: Vec<CommandNote>,
    categories: Vec<Category>,
    category_counts: HashMap<i64, i64>,
    trash: Vec<(CommandNote, chrono::DateTime<chrono::Utc>)>,

    // UI state
    selected_view: View,
    selected_command_id: Option<i64>,
    search: String,
    md_cache: CommonMarkCache,

    // inline editor for the right pane
    edit_buffer: Option<EditBuffer>,

    // modals
    quick_add: Option<QuickAddState>,
    new_category_name: String,
    show_new_category: bool,
    confirm_delete: Option<i64>,
    show_import: bool,
    import_error: Option<String>,

    // ephemeral
    toast: Option<Toast>,
    clipboard: Option<arboard::Clipboard>,
    pending_focus: PendingFocus,

    // LLM / AI
    llm: LlmManager,
    show_settings: bool,
    settings_draft: Option<LlmConfig>,
    settings_test_msg: Option<String>,
    settings_api_key_input: String,
    settings_api_key_visible: bool,
    nl_gen: Option<NlGenState>,
    pending_consent: Option<ConsentSubject>,

    // recycle bin + undo
    last_deleted: Option<i64>,
    show_shortcuts: bool,
    show_about: bool,
    fill_params: Option<FillParamsState>,
    update_checker: UpdateChecker,
}

#[derive(Default)]
struct FillParamsState {
    command_id: i64,
    template: String,
    vars: Vec<(String, String)>,
}

#[derive(Default)]
struct NlGenState {
    prompt: String,
    in_flight: bool,
}

#[derive(Debug, Clone, Copy)]
enum ConsentSubject {
    PasteEnhance,
}

#[derive(Default)]
struct PendingFocus {
    search: bool,
    quick_command: bool,
    editor_title: bool,
    scroll_to_selected: bool,
}

struct Toast {
    msg: String,
    color: Color32,
    expires: Instant,
}

#[derive(Default)]
struct QuickAddState {
    command: String,
    title: String,
    category_name: String,
    tags: Vec<String>,
    tag_input: String,
    description: String,
    last_inferred_from: String,
    duplicate_id: Option<i64>,
    // dirty flags — true when user manually edited the field
    title_dirty: bool,
    category_dirty: bool,
    tags_dirty: bool,
    desc_dirty: bool,
    // AI status
    enhance_status: EnhanceStatus,
    enhanced_from: Option<String>,
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
enum EnhanceStatus {
    #[default]
    Idle,
    Pending,
    Done,
    Failed,
}

/// In-flight inline edits for the currently-selected command. Auto-saves
/// after 500ms of inactivity. Created on selection change, flushed when
/// selection changes again or when the app exits.
struct EditBuffer {
    command_id: i64,
    title: String,
    command: String,
    description: String,
    category_id: Option<i64>,
    tags: Vec<String>,
    favorite: bool,
    tag_input: String,
    desc_editing: bool,
    describing: bool,
    dirty: bool,
    last_change_at: Option<Instant>,
}

impl App {
    pub fn new(cc: &CreationContext<'_>) -> Self {
        let db = Db::open().expect("failed to open database");
        // Purge anything trashed > 7 days on startup
        if let Ok(n) = db.purge_old_deleted(7) {
            if n > 0 {
                log::info!("purged {n} stale soft-deleted commands");
            }
        }
        let llm = LlmManager::load();
        let prefs: Prefs = cc
            .storage
            .and_then(|s| eframe::get_value::<Prefs>(s, PREFS_KEY))
            .unwrap_or_default();
        let mut app = Self {
            db,
            engine: SearchEngine::default(),
            commands: Vec::new(),
            categories: Vec::new(),
            category_counts: HashMap::new(),
            trash: Vec::new(),
            selected_view: prefs.view,
            selected_command_id: prefs.selected_command_id,
            search: prefs.search,
            md_cache: CommonMarkCache::default(),
            edit_buffer: None,
            quick_add: None,
            new_category_name: String::new(),
            show_new_category: false,
            confirm_delete: None,
            show_import: false,
            import_error: None,
            toast: None,
            clipboard: arboard::Clipboard::new().ok(),
            pending_focus: PendingFocus::default(),
            llm,
            show_settings: false,
            settings_draft: None,
            settings_test_msg: None,
            settings_api_key_input: String::new(),
            settings_api_key_visible: false,
            nl_gen: None,
            pending_consent: None,
            last_deleted: None,
            show_shortcuts: false,
            show_about: false,
            fill_params: None,
            update_checker: UpdateChecker::new(),
        };
        app.reload();
        app
    }

    fn reload(&mut self) {
        if let Ok(c) = self.db.list_categories() {
            self.categories = c;
        }
        if let Ok(cmds) = self.db.list_commands() {
            self.commands = cmds;
        }
        if let Ok(map) = self.db.category_counts() {
            self.category_counts = map;
        }
        if let Ok(t) = self.db.list_trashed() {
            self.trash = t;
        }
    }

    fn toast_success(&mut self, msg: impl Into<String>) {
        self.toast = Some(Toast {
            msg: msg.into(),
            color: Color32::from_rgb(60, 160, 90),
            expires: Instant::now() + Duration::from_millis(1800),
        });
    }

    fn toast_error(&mut self, msg: impl Into<String>) {
        self.toast = Some(Toast {
            msg: msg.into(),
            color: Color32::from_rgb(200, 80, 80),
            expires: Instant::now() + Duration::from_secs(4),
        });
    }

    fn copy_to_clipboard(&mut self, text: &str) {
        if let Some(cb) = self.clipboard.as_mut() {
            match cb.set_text(text.to_string()) {
                Ok(_) => self.toast_success("已复制"),
                Err(e) => self.toast_error(format!("复制失败: {e}")),
            }
        } else {
            self.toast_error("剪贴板不可用");
        }
    }

    fn category_name(&self, id: Option<i64>) -> &str {
        match id {
            None => "Other",
            Some(cid) => self
                .categories
                .iter()
                .find(|c| c.id == cid)
                .map(|c| c.name.as_str())
                .unwrap_or("Other"),
        }
    }

    fn ensure_category_id(&mut self, name: &str) -> Option<i64> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        if let Some(c) = self.categories.iter().find(|c| c.name.eq_ignore_ascii_case(name)) {
            return Some(c.id);
        }
        match self.db.add_category(name) {
            Ok(id) => {
                self.reload();
                Some(id)
            }
            Err(_) => None,
        }
    }

    fn filtered_commands(&self) -> Vec<&CommandNote> {
        // Trash view: pull from a different source and skip category/favorite filter.
        if self.selected_view == View::Trash {
            let mut list: Vec<&CommandNote> = self.trash.iter().map(|(c, _)| c).collect();
            if !self.search.trim().is_empty() {
                let mut scored: Vec<(i64, &CommandNote)> = list
                    .iter()
                    .filter_map(|c| {
                        let s = self.engine.score(&self.search, c, "Trash");
                        if s > 0 {
                            Some((s, *c))
                        } else {
                            None
                        }
                    })
                    .collect();
                scored.sort_by(|a, b| b.0.cmp(&a.0));
                list = scored.into_iter().map(|(_, c)| c).collect();
            }
            return list;
        }

        let mut filtered: Vec<&CommandNote> = self
            .commands
            .iter()
            .filter(|c| match self.selected_view {
                View::All => true,
                View::Favorites => c.favorite,
                View::Recent => c.visit_count > 0,
                View::Category(cid) => c.category_id == Some(cid),
                View::Trash => false, // unreachable, handled above
            })
            .collect();

        if self.selected_view == View::Recent {
            filtered.sort_by(|a, b| {
                b.visit_count
                    .cmp(&a.visit_count)
                    .then_with(|| b.updated_at.cmp(&a.updated_at))
            });
            filtered.truncate(20);
        }

        if self.search.trim().is_empty() {
            return filtered;
        }

        let mut scored: Vec<(i64, &CommandNote)> = filtered
            .into_iter()
            .filter_map(|c| {
                let cat = self.category_name(c.category_id).to_string();
                let s = self.engine.score(&self.search, c, &cat);
                if s > 0 {
                    Some((s, c))
                } else {
                    None
                }
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        scored.into_iter().map(|(_, c)| c).collect()
    }

    #[allow(dead_code)]
    fn selected_command(&self) -> Option<&CommandNote> {
        let id = self.selected_command_id?;
        self.commands
            .iter()
            .find(|c| c.id == id)
            .or_else(|| self.trash.iter().find(|(c, _)| c.id == id).map(|(c, _)| c))
    }

    // ---------- inline edit buffer ----------

    /// At the start of every frame: if selection changed since last buffer
    /// was loaded, flush pending edits and load a fresh buffer.
    fn sync_edit_buffer(&mut self) {
        let buffer_id = self.edit_buffer.as_ref().map(|b| b.command_id);
        if buffer_id != self.selected_command_id {
            self.flush_edit_buffer();
            self.load_edit_buffer();
        }
    }

    fn load_edit_buffer(&mut self) {
        let Some(id) = self.selected_command_id else {
            self.edit_buffer = None;
            return;
        };
        let note = self
            .commands
            .iter()
            .find(|c| c.id == id)
            .cloned()
            .or_else(|| self.trash.iter().find(|(c, _)| c.id == id).cloned().map(|(c, _)| c));
        self.edit_buffer = note.map(|c| EditBuffer {
            command_id: c.id,
            title: c.title,
            command: c.command,
            description: c.description,
            category_id: c.category_id,
            tags: c.tags,
            favorite: c.favorite,
            tag_input: String::new(),
            desc_editing: false,
            describing: false,
            dirty: false,
            last_change_at: None,
        });
    }

    /// Write the buffer to DB if there are pending changes.
    fn flush_edit_buffer(&mut self) {
        let Some(b) = self.edit_buffer.as_ref() else { return };
        if !b.dirty {
            return;
        }
        // Skip if the source is in trash — trashed items aren't editable
        if self.is_trashed(b.command_id) {
            if let Some(buf) = self.edit_buffer.as_mut() {
                buf.dirty = false;
                buf.last_change_at = None;
            }
            return;
        }
        if let Some(mut note) = self.commands.iter().find(|c| c.id == b.command_id).cloned() {
            note.title = if b.title.trim().is_empty() {
                note.command.lines().next().unwrap_or("Untitled").to_string()
            } else {
                b.title.trim().to_string()
            };
            note.command = b.command.clone();
            note.description = b.description.clone();
            note.category_id = b.category_id;
            note.tags = b.tags.clone();
            note.favorite = b.favorite;
            if let Err(e) = self.db.update_command(&note) {
                self.toast_error(format!("自动保存失败: {e}"));
                return;
            }
            self.reload();
        }
        if let Some(buf) = self.edit_buffer.as_mut() {
            buf.dirty = false;
            buf.last_change_at = None;
        }
    }

    fn mark_buffer_dirty(&mut self) {
        if let Some(b) = self.edit_buffer.as_mut() {
            b.dirty = true;
            b.last_change_at = Some(Instant::now());
        }
    }

    /// Auto-save buffer 500ms after last edit.
    fn tick_autosave(&mut self) {
        let should_save = self
            .edit_buffer
            .as_ref()
            .and_then(|b| {
                if b.dirty {
                    b.last_change_at.map(|t| t.elapsed() >= Duration::from_millis(500))
                } else {
                    None
                }
            })
            .unwrap_or(false);
        if should_save {
            self.flush_edit_buffer();
        }
    }

    fn is_trashed(&self, id: i64) -> bool {
        self.trash.iter().any(|(c, _)| c.id == id)
    }

    fn trash_deleted_at(&self, id: i64) -> Option<chrono::DateTime<chrono::Utc>> {
        self.trash
            .iter()
            .find(|(c, _)| c.id == id)
            .map(|(_, t)| *t)
    }

    // ---------- actions ----------

    fn open_quick_add(&mut self) {
        let clip = self
            .clipboard
            .as_mut()
            .and_then(|cb| cb.get_text().ok())
            .unwrap_or_default();
        let mut st = QuickAddState::default();
        if !clip.trim().is_empty() {
            st.command = clip;
            self.reinfer_quick_add(&mut st);
        }
        self.quick_add = Some(st);
        self.pending_focus.quick_command = true;
    }

    fn reinfer_quick_add(&self, st: &mut QuickAddState) {
        if st.command == st.last_inferred_from {
            return;
        }
        let r = inference::infer(&st.command);
        // Only fill fields that the user hasn't manually touched (heuristic: empty)
        if st.title.is_empty() {
            st.title = r.title;
        }
        if st.category_name.is_empty() {
            st.category_name = r.category;
        }
        if st.tags.is_empty() {
            st.tags = r.tags;
        }
        st.last_inferred_from = st.command.clone();
    }

    fn save_quick_add(&mut self) {
        let Some(mut st) = self.quick_add.take() else {
            return;
        };
        // Re-infer once more in case command changed and fields are still empty
        self.reinfer_quick_add(&mut st);

        // Use inferred command body if user pasted a `# comment\ncmd` form
        let parsed = inference::infer(&st.command);
        let command_text = if parsed.command.is_empty() {
            st.command.trim().to_string()
        } else {
            parsed.command
        };
        let title = if st.title.trim().is_empty() {
            command_text.lines().next().unwrap_or("Untitled").trim().to_string()
        } else {
            st.title.trim().to_string()
        };

        if command_text.is_empty() {
            self.toast_error("命令不能为空");
            self.quick_add = Some(st);
            return;
        }

        // Duplicate check
        if let Ok(Some(existing)) = self.db.find_by_exact_command(&command_text) {
            self.selected_command_id = Some(existing.id);
            self.toast_error("命令已存在，已为你打开原条目");
            return;
        }

        let category_id = if st.category_name.trim().is_empty() {
            self.ensure_category_id("Other")
        } else {
            self.ensure_category_id(&st.category_name)
        };

        let mut note = CommandNote::new(title, command_text);
        note.description = st.description.clone();
        note.category_id = category_id;
        note.tags = st.tags.clone();

        match self.db.insert_command(&note) {
            Ok(id) => {
                self.reload();
                self.selected_command_id = Some(id);
                self.toast_success("已保存");
            }
            Err(e) => {
                self.toast_error(format!("保存失败: {e}"));
                self.quick_add = Some(st);
            }
        }
    }

    /// Select a command and focus its title input in the detail pane.
    fn focus_detail_title(&mut self, id: i64) {
        self.selected_command_id = Some(id);
        self.pending_focus.editor_title = true;
    }

    /// Soft delete — moves the command into the recycle bin. Caller can undo
    /// via the toast or restore from the Trash view (auto-purged after 7 days).
    fn delete_command(&mut self, id: i64) {
        match self.db.soft_delete_command(id) {
            Ok(_) => {
                self.reload();
                if self.selected_command_id == Some(id) {
                    self.selected_command_id = None;
                }
                self.last_deleted = Some(id);
                self.toast = Some(Toast {
                    msg: "已删除 · 点击撤销".into(),
                    color: Color32::from_rgb(80, 80, 100),
                    expires: Instant::now() + Duration::from_secs(6),
                });
            }
            Err(e) => self.toast_error(format!("删除失败: {e}")),
        }
    }

    fn restore_command(&mut self, id: i64) {
        if let Err(e) = self.db.restore_command(id) {
            self.toast_error(format!("还原失败: {e}"));
            return;
        }
        self.reload();
        self.selected_command_id = Some(id);
        self.last_deleted = None;
        self.toast_success("已还原");
    }

    fn hard_delete_command(&mut self, id: i64) {
        if let Err(e) = self.db.delete_command(id) {
            self.toast_error(format!("永久删除失败: {e}"));
            return;
        }
        self.reload();
        if self.selected_command_id == Some(id) {
            self.selected_command_id = None;
        }
        self.toast_success("已永久删除");
    }

    fn undo_last_delete(&mut self) {
        if let Some(id) = self.last_deleted.take() {
            self.restore_command(id);
        }
    }

    fn toggle_favorite(&mut self, id: i64) {
        if let Err(e) = self.db.toggle_favorite(id) {
            self.toast_error(format!("操作失败: {e}"));
        } else {
            self.reload();
        }
    }

    fn copy_command(&mut self, id: i64) {
        let Some(c) = self
            .commands
            .iter()
            .find(|c| c.id == id)
            .or_else(|| self.trash.iter().find(|(c, _)| c.id == id).map(|(c, _)| c))
            .cloned()
        else {
            return;
        };
        let vars = parse_variables(&c.command);
        if vars.is_empty() {
            let _ = self.db.bump_visit(id);
            self.reload();
            self.copy_to_clipboard(&c.command);
        } else {
            self.fill_params = Some(FillParamsState {
                command_id: id,
                template: c.command.clone(),
                vars: vars.into_iter().map(|v| (v, String::new())).collect(),
            });
        }
    }

    fn finish_fill_and_copy(&mut self) {
        let Some(st) = self.fill_params.take() else { return };
        let filled = substitute_variables(&st.template, &st.vars);
        let _ = self.db.bump_visit(st.command_id);
        self.reload();
        self.copy_to_clipboard(&filled);
    }

    fn export_json(&mut self) {
        let bundle = ExportBundle {
            version: 1,
            exported_at: chrono::Utc::now(),
            categories: self.categories.clone(),
            commands: self.commands.clone(),
        };
        let json = match serde_json::to_string_pretty(&bundle) {
            Ok(s) => s,
            Err(e) => {
                self.toast_error(format!("导出失败: {e}"));
                return;
            }
        };
        let dir = match crate::db::data_dir() {
            Ok(d) => d,
            Err(e) => {
                self.toast_error(format!("找不到数据目录: {e}"));
                return;
            }
        };
        let path = dir.join(format!(
            "spellbook-export-{}.json",
            chrono::Utc::now().format("%Y%m%d-%H%M%S")
        ));
        if let Err(e) = std::fs::write(&path, json) {
            self.toast_error(format!("写文件失败: {e}"));
            return;
        }
        self.toast_success(format!("已导出: {}", path.display()));
    }

    fn import_json_from_text(&mut self, text: &str) -> Result<usize> {
        let bundle: ExportBundle = serde_json::from_str(text)?;
        let mut cat_map: HashMap<i64, i64> = HashMap::new();
        for old in &bundle.categories {
            let new_id = self.ensure_category_id(&old.name).unwrap_or(0);
            cat_map.insert(old.id, new_id);
        }
        let mut count = 0;
        for c in &bundle.commands {
            // skip if exact command already exists
            if let Ok(Some(_)) = self.db.find_by_exact_command(&c.command) {
                continue;
            }
            let mut note = c.clone();
            note.id = 0;
            note.category_id = c.category_id.and_then(|id| cat_map.get(&id).copied());
            if self.db.insert_command(&note).is_ok() {
                count += 1;
            }
        }
        self.reload();
        Ok(count)
    }
}

// ============================================================================
// egui::App implementation
// ============================================================================

impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.flush_edit_buffer();
        let prefs = Prefs {
            view: self.selected_view,
            selected_command_id: self.selected_command_id,
            search: self.search.clone(),
        };
        eframe::set_value(storage, PREFS_KEY, &prefs);
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Drain LLM + update-check results from worker threads.
        let results = self.llm.poll();
        for r in results {
            self.handle_llm_result(r);
        }
        self.update_checker.poll();

        // Inline-editor lifecycle: swap buffer when selection changes,
        // and debounced auto-save on changes.
        self.sync_edit_buffer();
        self.tick_autosave();

        self.handle_shortcuts(ctx);

        self.draw_top_bar(ctx);
        self.draw_left_sidebar(ctx);
        self.draw_command_detail(ctx);
        self.draw_command_list(ctx);

        self.draw_quick_add(ctx);
        self.draw_new_category(ctx);
        self.draw_confirm_delete(ctx);
        self.draw_import(ctx);
        self.draw_settings(ctx);
        self.draw_nl_gen(ctx);
        self.draw_consent(ctx);
        self.draw_fill_params(ctx);
        self.draw_shortcuts(ctx);
        self.draw_about(ctx);
        self.draw_toast(ctx);

        // expire toast
        if let Some(t) = &self.toast {
            if t.expires <= Instant::now() {
                self.toast = None;
            } else {
                ctx.request_repaint_after(Duration::from_millis(200));
            }
        }

        // keep ticking while LLM jobs are running so we pick up results promptly
        if self.llm.in_flight() {
            ctx.request_repaint_after(Duration::from_millis(150));
        }
    }
}

impl App {
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let (cmd_n, cmd_k, cmd_shift_c, cmd_d, esc, cmd_e, cmd_i) = ctx.input(|i| {
            (
                i.key_pressed(Key::N) && i.modifiers.command,
                i.key_pressed(Key::K) && i.modifiers.command,
                i.key_pressed(Key::C) && i.modifiers.command && i.modifiers.shift,
                i.key_pressed(Key::D) && i.modifiers.command,
                i.key_pressed(Key::Escape),
                i.key_pressed(Key::E) && i.modifiers.command,
                i.key_pressed(Key::I) && i.modifiers.command,
            )
        });

        if cmd_n && self.quick_add.is_none() {
            self.open_quick_add();
        }
        if cmd_k {
            self.pending_focus.search = true;
        }
        if cmd_shift_c {
            if let Some(id) = self.selected_command_id {
                self.copy_command(id);
            }
        }
        if cmd_d {
            if let Some(id) = self.selected_command_id {
                self.toggle_favorite(id);
            }
        }
        if cmd_e {
            if let Some(id) = self.selected_command_id {
                self.focus_detail_title(id);
            }
        }
        if cmd_i && self.nl_gen.is_none() && self.quick_add.is_none() {
            self.nl_gen = Some(NlGenState::default());
        }

        // List navigation (↑↓ Enter). Only active when no modal is open
        // (modals would absorb the keys anyway, but we guard explicitly).
        let modal_open = self.quick_add.is_some()
            || self.nl_gen.is_some()
            || self.show_settings
            || self.pending_consent.is_some()
            || self.show_new_category
            || self.confirm_delete.is_some()
            || self.show_import
            || self.show_shortcuts
            || self.show_about
            || self.fill_params.is_some();
        if !modal_open {
            let (up, down, enter) = ctx.input(|i| {
                (
                    i.key_pressed(Key::ArrowDown),
                    i.key_pressed(Key::ArrowUp),
                    i.key_pressed(Key::Enter)
                        && !i.modifiers.shift
                        && !i.modifiers.command,
                )
            });
            // up = first tuple element due to order above (down, up, enter)
            // re-bind for clarity:
            let (down_pressed, up_pressed, enter_pressed) = (up, down, enter);
            if up_pressed || down_pressed {
                let list: Vec<i64> =
                    self.filtered_commands().iter().map(|c| c.id).collect();
                if !list.is_empty() {
                    let cur = self
                        .selected_command_id
                        .and_then(|id| list.iter().position(|&x| x == id));
                    let new_idx = match cur {
                        None => 0,
                        Some(i) if down_pressed => (i + 1).min(list.len() - 1),
                        Some(i) if up_pressed => i.saturating_sub(1),
                        Some(i) => i,
                    };
                    self.selected_command_id = Some(list[new_idx]);
                    self.pending_focus.scroll_to_selected = true;
                }
            }
            if enter_pressed {
                if let Some(id) = self.selected_command_id {
                    if self.is_trashed(id) {
                        self.restore_command(id);
                    } else {
                        self.copy_command(id);
                    }
                }
            }
        }

        // Help panel: ? or F1
        let help = ctx.input(|i| {
            (i.key_pressed(Key::F1)) || (i.key_pressed(Key::Slash) && i.modifiers.shift)
        });
        if help && !modal_open {
            self.show_shortcuts = !self.show_shortcuts;
        }

        if esc {
            if self.fill_params.is_some() {
                self.fill_params = None;
            } else if self.show_about {
                self.show_about = false;
            } else if self.show_shortcuts {
                self.show_shortcuts = false;
            } else if self.quick_add.is_some() {
                self.quick_add = None;
            } else if self.nl_gen.is_some() {
                self.nl_gen = None;
            } else if self.show_settings {
                self.show_settings = false;
                self.settings_draft = None;
            } else if self.pending_consent.is_some() {
                self.pending_consent = None;
            } else if self.show_new_category {
                self.show_new_category = false;
            } else if self.confirm_delete.is_some() {
                self.confirm_delete = None;
            } else if self.show_import {
                self.show_import = false;
            } else if !self.search.is_empty() {
                self.search.clear();
            }
        }
    }

    // ---------- top bar ----------

    fn draw_top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                // Brand + version (click → About modal)
                let title = RichText::new("📚 Spellbook").strong();
                let ver = format!("v{}", self.update_checker.current_version());
                let brand = ui
                    .add(egui::Label::new(title).sense(egui::Sense::click()))
                    .on_hover_text("点击查看版本 / 检查更新");
                if brand.clicked() {
                    self.show_about = true;
                }
                let ver_text = RichText::new(&ver).small().weak();
                let ver_resp = ui
                    .add(egui::Label::new(ver_text).sense(egui::Sense::click()))
                    .on_hover_text("点击查看版本 / 检查更新");
                if ver_resp.clicked() {
                    self.show_about = true;
                }
                if self.update_checker.has_update() {
                    let new_v = self
                        .update_checker
                        .update_info()
                        .map(|i| i.latest_version.clone())
                        .unwrap_or_default();
                    let badge = ui
                        .add(
                            egui::Label::new(
                                RichText::new(format!("↑ 新版 v{new_v}"))
                                    .small()
                                    .color(Color32::from_rgb(100, 200, 255)),
                            )
                            .sense(egui::Sense::click()),
                        )
                        .on_hover_text("点击查看");
                    if badge.clicked() {
                        self.show_about = true;
                    }
                }
                if self.llm.in_flight() {
                    ui.label(RichText::new("🧠 AI 处理中").color(Color32::from_rgb(120, 180, 240)));
                }
                ui.separator();

                let search_resp = ui.add_sized(
                    [ui.available_width() - 480.0, 24.0],
                    TextEdit::singleline(&mut self.search)
                        .hint_text("🔍  搜索  (Ctrl/Cmd+K)"),
                );
                if self.pending_focus.search {
                    search_resp.request_focus();
                    self.pending_focus.search = false;
                }

                if ui.button("✨ 新建 (Cmd+N)").clicked() {
                    self.open_quick_add();
                }
                if ui.button("🪄 AI 生成 (Cmd+I)").clicked() {
                    self.nl_gen = Some(NlGenState::default());
                }
                if ui.button("⚙ 设置").clicked() {
                    self.settings_draft = Some(self.llm.config.clone());
                    self.show_settings = true;
                    self.settings_test_msg = None;
                }
                if ui.button("⤓ 导入").clicked() {
                    self.show_import = true;
                }
                if ui.button("⤒ 导出").clicked() {
                    self.export_json();
                }
            });
            ui.add_space(4.0);
        });
    }

    // ---------- left sidebar ----------

    fn draw_left_sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("categories")
            .resizable(true)
            .default_width(200.0)
            .width_range(160.0..=320.0)
            .show(ctx, |ui| {
                ui.add_space(6.0);

                let all_count = self.commands.len();
                let fav_count = self.commands.iter().filter(|c| c.favorite).count();
                let recent_count = self
                    .commands
                    .iter()
                    .filter(|c| c.visit_count > 0)
                    .count()
                    .min(20);

                self.sidebar_entry(ui, View::All, "📋 全部", all_count);
                self.sidebar_entry(ui, View::Favorites, "★ 收藏", fav_count);
                self.sidebar_entry(ui, View::Recent, "🕘 最近", recent_count);
                let trash_count = self.trash.len();
                if trash_count > 0 {
                    self.sidebar_entry(ui, View::Trash, "🗑 回收站", trash_count);
                }

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("分类").weak());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("+").on_hover_text("新建分类").clicked() {
                            self.show_new_category = true;
                            self.new_category_name.clear();
                        }
                    });
                });
                ui.add_space(2.0);

                ScrollArea::vertical().show(ui, |ui| {
                    let cats = self.categories.clone();
                    for cat in &cats {
                        let count = self.category_counts.get(&cat.id).copied().unwrap_or(0) as usize;
                        self.sidebar_entry(ui, View::Category(cat.id), &cat.name, count);
                    }
                });
            });
    }

    fn sidebar_entry(&mut self, ui: &mut Ui, view: View, label: &str, count: usize) {
        let selected = self.selected_view == view;
        let text = format!("{}  ({})", label, count);
        let resp = ui.selectable_label(selected, text);
        if resp.clicked() {
            self.selected_view = view;
            self.selected_command_id = None;
        }
        resp.context_menu(|ui| {
            if let View::Category(cid) = view {
                let cat_id = cid;
                if ui.button("重命名").clicked() {
                    if let Some(c) = self.categories.iter().find(|c| c.id == cat_id) {
                        self.new_category_name = c.name.clone();
                        self.show_new_category = true;
                    }
                    ui.close_menu();
                }
                if ui.button(RichText::new("🗑 删除").color(Color32::from_rgb(200, 80, 80))).clicked() {
                    let _ = self.db.delete_category(cat_id);
                    self.reload();
                    if self.selected_view == View::Category(cat_id) {
                        self.selected_view = View::All;
                    }
                    ui.close_menu();
                }
            }
        });
    }

    // ---------- detail (right) ----------

    fn draw_command_detail(&mut self, ctx: &egui::Context) {
        egui::SidePanel::right("detail")
            .resizable(true)
            .default_width(460.0)
            .width_range(320.0..=900.0)
            .show(ctx, |ui| {
                ui.add_space(6.0);
                if self.edit_buffer.is_none() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(40.0);
                        ui.label(RichText::new("👈 选一条命令直接编辑").weak());
                        ui.add_space(8.0);
                        ui.label(RichText::new("或按 Cmd+N 极速新建").weak());
                    });
                    return;
                }

                let cmd_id = self.edit_buffer.as_ref().unwrap().command_id;
                let trashed = self.is_trashed(cmd_id);
                if trashed {
                    self.draw_detail_trash_view(ui, cmd_id);
                } else {
                    self.draw_detail_edit_view(ui, cmd_id);
                }
            });
    }

    /// Inline editor for an active command (auto-saves every 500ms after last edit).
    fn draw_detail_edit_view(&mut self, ui: &mut Ui, cmd_id: i64) {
        // 1) Top row: ★ + Title (inline) + save status
        let star = {
            let b = self.edit_buffer.as_ref().unwrap();
            if b.favorite { "★" } else { "☆" }
        };
        ui.horizontal(|ui| {
            if ui.button(star).on_hover_text("收藏 (Cmd+D)").clicked() {
                if let Some(b) = self.edit_buffer.as_mut() {
                    b.favorite = !b.favorite;
                    b.dirty = true;
                    b.last_change_at = Some(Instant::now());
                }
            }
            let title_resp = {
                let b = self.edit_buffer.as_mut().unwrap();
                ui.add_sized(
                    [ui.available_width() - 80.0, 28.0],
                    TextEdit::singleline(&mut b.title)
                        .hint_text("标题")
                        .font(egui::TextStyle::Heading),
                )
            };
            if title_resp.changed() {
                self.mark_buffer_dirty();
            }
            if self.pending_focus.editor_title {
                title_resp.request_focus();
                self.pending_focus.editor_title = false;
            }
            // save indicator
            let dirty = self.edit_buffer.as_ref().map(|b| b.dirty).unwrap_or(false);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let label = if dirty { "编辑中…" } else { "已保存" };
                let color = if dirty {
                    Color32::from_rgb(220, 180, 100)
                } else {
                    Color32::from_rgb(120, 180, 120)
                };
                ui.label(RichText::new(label).small().color(color));
            });
        });

        ui.add_space(2.0);

        // 2) Category + tags row
        let cat_names: Vec<(i64, String)> = self
            .categories
            .iter()
            .map(|c| (c.id, c.name.clone()))
            .collect();
        ui.horizontal(|ui| {
            let current_name = self
                .edit_buffer
                .as_ref()
                .and_then(|b| b.category_id)
                .and_then(|id| cat_names.iter().find(|(i, _)| *i == id).map(|(_, n)| n.clone()))
                .unwrap_or_else(|| "—".to_string());
            let mut new_cat_id: Option<Option<i64>> = None;
            egui::ComboBox::from_id_salt("detail_cat")
                .selected_text(current_name)
                .show_ui(ui, |ui| {
                    for (id, name) in &cat_names {
                        if ui
                            .selectable_label(
                                self.edit_buffer.as_ref().and_then(|b| b.category_id) == Some(*id),
                                name,
                            )
                            .clicked()
                        {
                            new_cat_id = Some(Some(*id));
                        }
                    }
                });
            if let Some(cid) = new_cat_id {
                if let Some(b) = self.edit_buffer.as_mut() {
                    b.category_id = cid;
                    b.dirty = true;
                    b.last_change_at = Some(Instant::now());
                }
            }
        });

        // Tags as chips with × and add input
        ui.add_space(2.0);
        let mut filter_by: Option<String> = None;
        let mut tag_dirty = false;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let mut remove_idx: Option<usize> = None;
            let tags_snapshot: Vec<String> = self
                .edit_buffer
                .as_ref()
                .map(|b| b.tags.clone())
                .unwrap_or_default();
            for (i, t) in tags_snapshot.iter().enumerate() {
                let r = tag_chip(ui, t, true, false);
                if r.remove_clicked {
                    remove_idx = Some(i);
                }
                if !r.remove_clicked && r.response.clicked() {
                    filter_by = Some(t.clone());
                }
                r.response.on_hover_text("点击：按此标签筛选 · × 删除");
            }
            if let Some(i) = remove_idx {
                if let Some(b) = self.edit_buffer.as_mut() {
                    b.tags.remove(i);
                    tag_dirty = true;
                }
            }
            // Add-tag input
            let (resp, add) = {
                let b = self.edit_buffer.as_mut().unwrap();
                let r = ui.add(
                    TextEdit::singleline(&mut b.tag_input)
                        .hint_text("+ 标签")
                        .desired_width(110.0),
                );
                let add = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                (r, add)
            };
            if add {
                let b = self.edit_buffer.as_mut().unwrap();
                let t = b.tag_input.trim().trim_start_matches('#').to_string();
                if !t.is_empty() && !b.tags.contains(&t) {
                    b.tags.push(t);
                    tag_dirty = true;
                }
                b.tag_input.clear();
                resp.request_focus();
            }
        });
        if tag_dirty {
            self.mark_buffer_dirty();
        }
        if let Some(t) = filter_by {
            self.search = t;
            self.pending_focus.search = true;
        }

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        // 3) Command editor (multi-line, monospace)
        let cmd_resp = {
            let b = self.edit_buffer.as_mut().unwrap();
            let rows = b.command.lines().count().max(2).min(8);
            ui.add(
                TextEdit::multiline(&mut b.command)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY)
                    .desired_rows(rows),
            )
        };
        if cmd_resp.changed() {
            self.mark_buffer_dirty();
        }

        ui.add_space(6.0);

        // 4) Actions row
        ui.horizontal(|ui| {
            if ui
                .button(RichText::new("📋 复制").strong())
                .on_hover_text("Cmd+Shift+C")
                .clicked()
            {
                // flush pending edits first so we copy the latest
                self.flush_edit_buffer();
                self.copy_command(cmd_id);
            }
            if self.llm.config.features.explain_command
                && self.llm.is_configured()
                && !self.llm.config.offline_mode
            {
                let busy = self.llm.is_busy(JobKind::Explain);
                let label = if busy { "🧠 解释中…" } else { "🧠 AI 解释" };
                if ui.add_enabled(!busy, egui::Button::new(label)).clicked() {
                    let command = self
                        .edit_buffer
                        .as_ref()
                        .map(|b| b.command.clone())
                        .unwrap_or_default();
                    self.flush_edit_buffer();
                    self.llm.explain(&command, cmd_id);
                }
            }
            if ui
                .button(RichText::new("🗑 删除").color(Color32::from_rgb(200, 80, 80)))
                .clicked()
            {
                self.confirm_delete = Some(cmd_id);
            }
        });

        ui.add_space(10.0);

        // 5) Description: toggle between rendered Markdown and editor + AI generate
        let (desc_editing, describing, has_desc) = {
            let b = self.edit_buffer.as_ref().unwrap();
            (b.desc_editing, b.describing, !b.description.trim().is_empty())
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new("说明 (Markdown)").weak());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.llm.config.features.generate_description
                    && self.llm.is_configured()
                    && !self.llm.config.offline_mode
                {
                    let label = if describing { "🧠 生成中…" } else { "✨ AI 生成" };
                    if ui
                        .add_enabled(!describing, egui::Button::new(label).small())
                        .clicked()
                    {
                        let (title, command) = {
                            let b = self.edit_buffer.as_ref().unwrap();
                            (b.title.clone(), b.command.clone())
                        };
                        if let Some(b) = self.edit_buffer.as_mut() {
                            b.describing = true;
                        }
                        self.llm.generate_description(&title, &command, Some(cmd_id));
                    }
                }
                let toggle_label = if desc_editing { "👁 预览" } else { "✎ 编辑" };
                if ui.small_button(toggle_label).clicked() {
                    if let Some(b) = self.edit_buffer.as_mut() {
                        b.desc_editing = !b.desc_editing;
                    }
                }
            });
        });

        ui.add_space(2.0);
        if desc_editing {
            let resp = {
                let b = self.edit_buffer.as_mut().unwrap();
                ui.add(
                    TextEdit::multiline(&mut b.description)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .desired_rows(10),
                )
            };
            if resp.changed() {
                self.mark_buffer_dirty();
            }
        } else if has_desc {
            let description = self.edit_buffer.as_ref().unwrap().description.clone();
            ScrollArea::vertical()
                .id_salt("desc_scroll")
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    CommonMarkViewer::new().show(ui, &mut self.md_cache, &description);
                });
        } else {
            ui.label(RichText::new("（无说明，点上方 ✎ 编辑 或 ✨ AI 生成）").weak());
        }
    }

    /// Read-only detail view for trashed items, with restore / permanent-delete.
    fn draw_detail_trash_view(&mut self, ui: &mut Ui, cmd_id: i64) {
        let (title, command, description, category_id, tags, favorite) = {
            let b = self.edit_buffer.as_ref().unwrap();
            (
                b.title.clone(),
                b.command.clone(),
                b.description.clone(),
                b.category_id,
                b.tags.clone(),
                b.favorite,
            )
        };
        ui.horizontal(|ui| {
            let star = if favorite { "★" } else { "☆" };
            ui.label(RichText::new(star).weak());
            ui.label(RichText::new(&title).heading().weak());
        });
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(RichText::new(self.category_name(category_id)).weak().small());
            for tag in &tags {
                let _ = tag_chip(ui, tag, false, true);
            }
        });

        ui.add_space(6.0);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.add(
                TextEdit::multiline(&mut command.clone())
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY)
                    .desired_rows(command.lines().count().max(2).min(10))
                    .interactive(false),
            );
        });

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui
                .button(RichText::new("↶ 还原").strong())
                .on_hover_text("把命令从回收站还原")
                .clicked()
            {
                self.restore_command(cmd_id);
            }
            if ui
                .button(RichText::new("🗑 永久删除").color(Color32::from_rgb(200, 80, 80)))
                .on_hover_text("不可恢复")
                .clicked()
            {
                self.confirm_delete = Some(cmd_id);
            }
            if let Some(dt) = self.trash_deleted_at(cmd_id) {
                let age = chrono::Utc::now().signed_duration_since(dt);
                let days_left = 7 - age.num_days();
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{} 天后自动清理", days_left.max(0)))
                            .small()
                            .weak(),
                    );
                });
            }
        });

        ui.add_space(10.0);
        if !description.trim().is_empty() {
            ScrollArea::vertical()
                .id_salt("trash_desc")
                .show(ui, |ui| {
                    CommonMarkViewer::new().show(ui, &mut self.md_cache, &description);
                });
        }
    }

    // ---------- command list (center) ----------

    fn draw_command_list(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(6.0);

            let list = self.filtered_commands();
            let total = list.len();
            ui.horizontal(|ui| {
                let label = match self.selected_view {
                    View::All => "📋 全部",
                    View::Favorites => "★ 收藏",
                    View::Recent => "🕘 最近",
                    View::Trash => "🗑 回收站",
                    View::Category(_) => self.category_name(match self.selected_view {
                        View::Category(id) => Some(id),
                        _ => None,
                    }),
                };
                ui.label(RichText::new(format!("{}  ·  {} 条", label, total)).weak());
            });
            ui.add_space(4.0);

            if total == 0 {
                ui.vertical_centered(|ui| {
                    ui.add_space(48.0);
                    if self.search.is_empty() {
                        ui.label(RichText::new("空空如也，按 Cmd+N 新建第一条命令").weak());
                    } else {
                        ui.label(RichText::new(format!("没有匹配「{}」的命令", self.search)).weak());
                    }
                });
                return;
            }

            // collect ids first to avoid borrow conflicts
            let ids_and_meta: Vec<(i64, String, String, String, bool, Vec<String>)> = list
                .iter()
                .map(|c| {
                    let cat = self.category_name(c.category_id).to_string();
                    (c.id, c.title.clone(), c.command.clone(), cat, c.favorite, c.tags.clone())
                })
                .collect();

            ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                let mut to_copy: Option<i64> = None;
                for (id, title, command, cat, fav, tags) in ids_and_meta {
                    let selected = self.selected_command_id == Some(id);
                    let resp = egui::Frame::group(ui.style())
                        .fill(if selected {
                            ui.visuals().selection.bg_fill
                        } else {
                            ui.visuals().widgets.noninteractive.bg_fill
                        })
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                if fav {
                                    ui.label("★");
                                }
                                ui.label(RichText::new(&title).strong());
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.label(RichText::new(&cat).weak().small());
                                });
                            });
                            ui.label(
                                RichText::new(first_line(&command, 100))
                                    .monospace()
                                    .color(Color32::from_gray(170)),
                            );
                            if !tags.is_empty() {
                                ui.horizontal_wrapped(|ui| {
                                    ui.spacing_mut().item_spacing.x = 4.0;
                                    for tag in tags.iter().take(6) {
                                        let _ = tag_chip(ui, tag, false, true);
                                    }
                                });
                            }
                        })
                        .response
                        .interact(egui::Sense::click());

                    if selected && self.pending_focus.scroll_to_selected {
                        resp.scroll_to_me(Some(Align::Center));
                    }
                    if resp.clicked() {
                        self.selected_command_id = Some(id);
                    }
                    if resp.double_clicked() {
                        to_copy = Some(id);
                    }
                    resp.context_menu(|ui| {
                        if ui.button("📋 复制").clicked() {
                            to_copy = Some(id);
                            ui.close_menu();
                        }
                        if ui.button("✎ 编辑标题").clicked() {
                            self.focus_detail_title(id);
                            ui.close_menu();
                        }
                        let label = if fav { "取消收藏" } else { "★ 收藏" };
                        if ui.button(label).clicked() {
                            self.toggle_favorite(id);
                            ui.close_menu();
                        }
                        if ui
                            .button(RichText::new("🗑 删除").color(Color32::from_rgb(200, 80, 80)))
                            .clicked()
                        {
                            self.confirm_delete = Some(id);
                            ui.close_menu();
                        }
                    });
                    ui.add_space(4.0);
                }
                if let Some(id) = to_copy {
                    self.copy_command(id);
                }
            });
        });
        // consume the scroll request after one pass
        self.pending_focus.scroll_to_selected = false;
    }

    // ---------- quick add modal ----------

    fn draw_quick_add(&mut self, ctx: &egui::Context) {
        let Some(mut st) = self.quick_add.take() else {
            return;
        };
        let mut keep_open = true;
        let mut close_requested = false;
        let mut do_save = false;

        egui::Window::new("✨ 新建命令")
            .open(&mut keep_open)
            .collapsible(false)
            .resizable(true)
            .default_width(560.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(RichText::new("粘贴命令，其他字段自动识别 (Cmd+Enter 保存, Esc 取消)").weak());
                ui.add_space(6.0);

                let cmd_resp = ui.add(
                    TextEdit::multiline(&mut st.command)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("# 可选注释（用作标题）\ndocker container prune -f")
                        .desired_width(f32::INFINITY)
                        .desired_rows(4),
                );
                if self.pending_focus.quick_command {
                    cmd_resp.request_focus();
                    self.pending_focus.quick_command = false;
                }

                // re-infer when command changes
                if st.command != st.last_inferred_from {
                    let r = inference::infer(&st.command);
                    // Command changed -> reset all dirty flags and refill from rules
                    st.title_dirty = false;
                    st.tags_dirty = false;
                    st.category_dirty = false;
                    st.desc_dirty = false;
                    st.title = r.title.clone();
                    st.category_name = r.category.clone();
                    st.tags = r.tags.clone();
                    st.last_inferred_from = st.command.clone();
                    st.enhance_status = EnhanceStatus::Idle;

                    // duplicate detection
                    let parsed = inference::infer(&st.command);
                    let needle = if parsed.command.is_empty() {
                        st.command.trim().to_string()
                    } else {
                        parsed.command
                    };
                    st.duplicate_id = self
                        .db
                        .find_by_exact_command(&needle)
                        .ok()
                        .flatten()
                        .map(|c| c.id);

                    // Try AI enhance (sync if cached, async otherwise; no-op if disabled)
                    if !needle.is_empty() {
                        match self.llm.maybe_enhance(
                            &needle,
                            &st.title,
                            &st.category_name,
                            &st.tags,
                        ) {
                            EnhanceTrigger::Spawned => {
                                st.enhance_status = EnhanceStatus::Pending;
                                st.enhanced_from = Some(needle);
                            }
                            EnhanceTrigger::Cached(cached) => {
                                if !st.title_dirty {
                                    if let Some(t) = &cached.title {
                                        st.title = t.clone();
                                    }
                                }
                                if !st.tags_dirty && !cached.tags.is_empty() {
                                    st.tags = cached.tags.clone();
                                }
                                if !st.desc_dirty {
                                    if let Some(d) = &cached.description {
                                        st.description = d.clone();
                                    }
                                }
                                st.enhance_status = EnhanceStatus::Done;
                                st.enhanced_from = Some(needle);
                            }
                            EnhanceTrigger::Skipped => {}
                        }
                    }
                }

                // AI status badge
                match st.enhance_status {
                    EnhanceStatus::Pending => {
                        ui.label(
                            RichText::new("🧠 AI 增强中…（不影响保存）")
                                .color(Color32::from_rgb(120, 180, 240)),
                        );
                    }
                    EnhanceStatus::Done => {
                        ui.label(
                            RichText::new("🧠 AI 已增强")
                                .color(Color32::from_rgb(140, 200, 140)),
                        );
                    }
                    EnhanceStatus::Failed => {
                        ui.label(
                            RichText::new("⚠ AI 增强失败，已回退到规则结果")
                                .color(Color32::from_rgb(200, 140, 60)),
                        );
                    }
                    EnhanceStatus::Idle => {}
                }

                if let Some(dup_id) = st.duplicate_id {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("⚠ 库里已有完全相同的命令")
                                .color(Color32::from_rgb(200, 140, 60)),
                        );
                        if ui.button("打开它").clicked() {
                            self.selected_command_id = Some(dup_id);
                            self.quick_add = None;
                        }
                    });
                }

                ui.add_space(6.0);
                ui.separator();
                ui.add_space(4.0);

                egui::Grid::new("quick_add_grid")
                    .num_columns(2)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("标题");
                        let r = ui.add(
                            TextEdit::singleline(&mut st.title).desired_width(f32::INFINITY),
                        );
                        if r.changed() {
                            st.title_dirty = true;
                        }
                        ui.end_row();

                        ui.label("分类");
                        let cat_names: Vec<String> =
                            self.categories.iter().map(|c| c.name.clone()).collect();
                        egui::ComboBox::from_id_salt("quick_add_cat")
                            .selected_text(if st.category_name.is_empty() {
                                "Other".to_string()
                            } else {
                                st.category_name.clone()
                            })
                            .show_ui(ui, |ui| {
                                for name in &cat_names {
                                    ui.selectable_value(&mut st.category_name, name.clone(), name);
                                }
                                ui.separator();
                                ui.horizontal(|ui| {
                                    ui.label("新建：");
                                    ui.text_edit_singleline(&mut st.category_name);
                                });
                            });
                        ui.end_row();

                        ui.label("标签");
                        ui.vertical(|ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                let mut remove: Option<usize> = None;
                                for (i, t) in st.tags.iter().enumerate() {
                                    let r = tag_chip(ui, t, true, false);
                                    if r.remove_clicked {
                                        remove = Some(i);
                                    }
                                }
                                if let Some(i) = remove {
                                    st.tags.remove(i);
                                    st.tags_dirty = true;
                                }
                            });
                            ui.horizontal(|ui| {
                                let resp = ui.add(
                                    TextEdit::singleline(&mut st.tag_input)
                                        .hint_text("+ 添加标签，回车确认")
                                        .desired_width(180.0),
                                );
                                let submit = resp.lost_focus()
                                    && ui.input(|i| i.key_pressed(Key::Enter));
                                if submit || ui.button("+").clicked() {
                                    let t = st.tag_input.trim().trim_start_matches('#').to_string();
                                    if !t.is_empty() && !st.tags.contains(&t) {
                                        st.tags.push(t);
                                        st.tags_dirty = true;
                                    }
                                    st.tag_input.clear();
                                    resp.request_focus();
                                }
                            });
                        });
                        ui.end_row();

                        ui.label("说明");
                        let r = ui.add(
                            TextEdit::multiline(&mut st.description)
                                .hint_text("可选，Markdown 格式，留空也行")
                                .desired_width(f32::INFINITY)
                                .desired_rows(4),
                        );
                        if r.changed() {
                            st.desc_dirty = true;
                        }
                        ui.end_row();
                    });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("💾 保存 (Cmd+Enter)").strong()).clicked() {
                        do_save = true;
                    }
                    if ui.button("取消 (Esc)").clicked() {
                        close_requested = true;
                    }
                });

                // ONLY Cmd/Ctrl+Enter saves. Bare Enter is always handed to
                // the focused TextEdit (newline in multiline, tag-submit in
                // singleline). Previous logic could accidentally fire save
                // when adding a newline in the command field.
                let _ = cmd_resp;
                let cmd_enter = ui.input(|i| {
                    i.key_pressed(Key::Enter) && i.modifiers.command
                });
                if cmd_enter {
                    do_save = true;
                }
            });

        if do_save {
            self.quick_add = Some(st);
            self.save_quick_add();
        } else if keep_open && !close_requested {
            self.quick_add = Some(st);
        }
    }

    // ---------- new category modal ----------

    fn draw_new_category(&mut self, ctx: &egui::Context) {
        if !self.show_new_category {
            return;
        }
        let mut open = true;
        let mut create = false;
        egui::Window::new("新建分类")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                let resp = ui.text_edit_singleline(&mut self.new_category_name);
                resp.request_focus();
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("创建").clicked() {
                        create = true;
                    }
                    if ui.button("取消").clicked() {
                        self.show_new_category = false;
                    }
                });
                if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    create = true;
                }
            });
        if !open {
            self.show_new_category = false;
        }
        if create {
            let name = self.new_category_name.trim().to_string();
            if !name.is_empty() {
                let _ = self.ensure_category_id(&name);
                self.show_new_category = false;
                self.toast_success("已创建分类");
            }
        }
    }

    // ---------- confirm delete modal ----------

    fn draw_confirm_delete(&mut self, ctx: &egui::Context) {
        let Some(id) = self.confirm_delete else {
            return;
        };
        // Look up in both live and trash
        let (c, is_hard) = match self.commands.iter().find(|c| c.id == id).cloned() {
            Some(c) => (c, false),
            None => match self.trash.iter().find(|(c, _)| c.id == id).cloned() {
                Some((c, _)) => (c, true),
                None => {
                    self.confirm_delete = None;
                    return;
                }
            },
        };
        let mut open = true;
        let mut do_delete = false;
        let title = if is_hard { "永久删除？" } else { "移到回收站？" };
        egui::Window::new(title)
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(RichText::new(&c.title).strong());
                ui.label(RichText::new(first_line(&c.command, 80)).monospace().weak());
                ui.add_space(6.0);
                if is_hard {
                    ui.label(
                        RichText::new("此操作不可撤销").color(Color32::from_rgb(200, 80, 80)),
                    );
                } else {
                    ui.label(RichText::new("7 天后自动彻底清理，期间可在回收站还原").weak());
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let label = if is_hard { "🗑 永久删除" } else { "🗑 删除" };
                    if ui
                        .button(RichText::new(label).color(Color32::from_rgb(200, 80, 80)))
                        .clicked()
                    {
                        do_delete = true;
                    }
                    if ui.button("取消").clicked() {
                        self.confirm_delete = None;
                    }
                });
            });
        if !open {
            self.confirm_delete = None;
        }
        if do_delete {
            self.confirm_delete = None;
            if is_hard {
                self.hard_delete_command(id);
            } else {
                self.delete_command(id);
            }
        }
    }

    // ---------- import dialog ----------

    fn draw_import(&mut self, ctx: &egui::Context) {
        if !self.show_import {
            return;
        }
        let mut open = true;
        let mut do_paste = false;
        let mut do_close = false;
        egui::Window::new("导入 JSON")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("从剪贴板粘贴 Spellbook 导出的 JSON：");
                ui.add_space(4.0);
                if let Some(err) = &self.import_error {
                    ui.label(
                        RichText::new(format!("⚠ {err}"))
                            .color(Color32::from_rgb(200, 80, 80)),
                    );
                    ui.add_space(4.0);
                }
                ui.horizontal(|ui| {
                    if ui.button("从剪贴板导入").clicked() {
                        do_paste = true;
                    }
                    if ui.button("取消").clicked() {
                        do_close = true;
                    }
                });
            });

        if !open || do_close {
            self.show_import = false;
            self.import_error = None;
            return;
        }
        if do_paste {
            let text = self
                .clipboard
                .as_mut()
                .and_then(|cb| cb.get_text().ok())
                .unwrap_or_default();
            if text.trim().is_empty() {
                self.import_error = Some("剪贴板为空".to_string());
            } else {
                match self.import_json_from_text(&text) {
                    Ok(n) => {
                        self.toast_success(format!("已导入 {n} 条"));
                        self.show_import = false;
                        self.import_error = None;
                    }
                    Err(e) => {
                        self.import_error = Some(format!("解析失败: {e}"));
                    }
                }
            }
        }
    }

    // ---------- About / update ----------

    fn draw_about(&mut self, ctx: &egui::Context) {
        if !self.show_about {
            return;
        }
        let mut open = true;
        let mut do_check = false;
        let mut copy_cli = false;
        let mut copy_cask = false;
        let mut open_release = false;

        let current = self.update_checker.current_version().to_string();
        let info = self.update_checker.update_info().cloned();
        let has_update = self.update_checker.has_update();
        let checking = self.update_checker.checking();
        let last_check = self.update_checker.last_check();
        let failed = self.update_checker.failed_reason().map(|s| s.to_string());

        egui::Window::new("关于 Spellbook")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("📚 Spellbook").heading());
                    ui.label(RichText::new(format!("v{current}")).weak());
                });
                ui.add_space(4.0);
                ui.label(RichText::new("Your spellbook of shell incantations").weak());

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(6.0);

                ui.label(RichText::new("更新").strong());
                ui.add_space(2.0);
                if checking {
                    ui.label(
                        RichText::new("正在检查最新版本…")
                            .color(Color32::from_rgb(120, 180, 240)),
                    );
                } else if let Some(info) = &info {
                    if has_update {
                        ui.label(
                            RichText::new(format!("✨ 新版可用：v{}", info.latest_version))
                                .color(Color32::from_rgb(100, 200, 255))
                                .strong(),
                        );
                        ui.add_space(6.0);
                        ui.label(RichText::new("Homebrew 升级命令").small().weak());
                        ui.horizontal(|ui| {
                            ui.add(
                                TextEdit::singleline(&mut "brew upgrade spellbook".to_string())
                                    .font(egui::TextStyle::Monospace)
                                    .desired_width(220.0)
                                    .interactive(false),
                            );
                            if ui.small_button("📋 复制").clicked() {
                                copy_cli = true;
                            }
                            ui.label(RichText::new("CLI").weak().small());
                        });
                        ui.horizontal(|ui| {
                            ui.add(
                                TextEdit::singleline(&mut "brew upgrade --cask spellbook".to_string())
                                    .font(egui::TextStyle::Monospace)
                                    .desired_width(220.0)
                                    .interactive(false),
                            );
                            if ui.small_button("📋 复制").clicked() {
                                copy_cask = true;
                            }
                            ui.label(RichText::new(".app").weak().small());
                        });
                        ui.add_space(6.0);
                        if ui
                            .button(RichText::new("🌐 打开 Release Notes"))
                            .clicked()
                        {
                            open_release = true;
                        }
                    } else {
                        ui.label(
                            RichText::new(format!("✓ 已是最新版 (v{})", info.latest_version))
                                .color(Color32::from_rgb(140, 200, 140)),
                        );
                    }
                    if let Some(ts) = last_check {
                        let age = chrono::Utc::now().signed_duration_since(ts);
                        let txt = if age.num_minutes() < 1 {
                            "刚刚检查".to_string()
                        } else if age.num_hours() < 1 {
                            format!("{} 分钟前检查", age.num_minutes())
                        } else if age.num_days() < 1 {
                            format!("{} 小时前检查", age.num_hours())
                        } else {
                            format!("{} 天前检查", age.num_days())
                        };
                        ui.label(RichText::new(txt).weak().small());
                    }
                } else if let Some(err) = failed {
                    ui.label(
                        RichText::new(format!("⚠ 检查失败: {err}"))
                            .color(Color32::from_rgb(200, 140, 60)),
                    );
                } else {
                    ui.label(RichText::new("尚未检查").weak());
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!checking, egui::Button::new("🔄 立即检查"))
                        .clicked()
                    {
                        do_check = true;
                    }
                    ui.hyperlink_to(
                        "GitHub 仓库",
                        "https://github.com/bkcarlos/spellbook",
                    );
                });

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(4.0);
                ui.label(RichText::new("数据位置").strong());
                if let Ok(dir) = crate::db::data_dir() {
                    ui.label(
                        RichText::new(dir.display().to_string())
                            .monospace()
                            .small()
                            .weak(),
                    );
                }
            });

        if do_check {
            self.update_checker.kick_off_check();
        }
        if copy_cli {
            self.copy_to_clipboard("brew upgrade spellbook");
        }
        if copy_cask {
            self.copy_to_clipboard("brew upgrade --cask spellbook");
        }
        if open_release {
            if let Some(info) = &info {
                if let Err(e) = open_url(&info.release_url) {
                    self.toast_error(format!("无法打开浏览器: {e}"));
                }
            }
        }
        if !open {
            self.show_about = false;
        }
    }

    // ---------- shortcut help ----------

    fn draw_shortcuts(&mut self, ctx: &egui::Context) {
        if !self.show_shortcuts {
            return;
        }
        let mut open = true;
        egui::Window::new("⌨ 快捷键")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                let rows = [
                    ("Ctrl/Cmd + K", "聚焦搜索框"),
                    ("Ctrl/Cmd + N", "极速新建（自动粘贴剪贴板）"),
                    ("Ctrl/Cmd + I", "AI 自然语言生成命令"),
                    ("Ctrl/Cmd + E", "编辑当前命令"),
                    ("Ctrl/Cmd + S", "保存编辑"),
                    ("Ctrl/Cmd + Shift + C", "复制当前命令"),
                    ("Ctrl/Cmd + D", "切换收藏"),
                    ("↑ / ↓", "在命令列表里上下移动"),
                    ("Enter", "复制当前选中命令（回收站中则还原）"),
                    ("Delete", "（在详情区）删除"),
                    ("Esc", "关闭弹窗 / 清空搜索"),
                    ("? 或 F1", "显示本面板"),
                ];
                egui::Grid::new("shortcut_grid")
                    .num_columns(2)
                    .spacing([16.0, 4.0])
                    .striped(true)
                    .show(ui, |ui| {
                        for (k, v) in rows {
                            ui.label(RichText::new(k).monospace().strong());
                            ui.label(v);
                            ui.end_row();
                        }
                    });
                ui.add_space(8.0);
                ui.label(RichText::new("提示: 命令里写 ${VAR} 占位符，复制前会弹框让你填值").weak());
            });
        if !open {
            self.show_shortcuts = false;
        }
    }

    // ---------- fill parameters ----------

    fn draw_fill_params(&mut self, ctx: &egui::Context) {
        let Some(mut st) = self.fill_params.take() else {
            return;
        };
        let mut keep_open = true;
        let mut close_requested = false;
        let mut do_copy = false;

        egui::Window::new("🔧 填充参数")
            .open(&mut keep_open)
            .collapsible(false)
            .resizable(true)
            .default_width(540.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(RichText::new("命令含 ${VAR} 占位符，填好就直接复制").weak());
                ui.add_space(6.0);
                ui.label(RichText::new("模板").small());
                ui.add(
                    TextEdit::multiline(&mut st.template.clone())
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .desired_rows(2)
                        .interactive(false),
                );

                ui.add_space(6.0);
                egui::Grid::new("fill_vars")
                    .num_columns(2)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        for (i, (name, value)) in st.vars.iter_mut().enumerate() {
                            ui.label(RichText::new(format!("${{{}}}", name)).monospace());
                            let resp = ui.add(
                                TextEdit::singleline(value)
                                    .desired_width(f32::INFINITY)
                                    .hint_text("值"),
                            );
                            if i == 0 {
                                resp.request_focus();
                            }
                            ui.end_row();
                        }
                    });

                ui.add_space(8.0);
                let preview = substitute_variables(&st.template, &st.vars);
                ui.label(RichText::new("预览").small());
                ui.add(
                    TextEdit::multiline(&mut preview.clone())
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .desired_rows(2)
                        .interactive(false),
                );

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let all_filled = st.vars.iter().all(|(_, v)| !v.trim().is_empty());
                    if ui
                        .add_enabled(
                            all_filled,
                            egui::Button::new(RichText::new("📋 复制 (Enter)").strong()),
                        )
                        .clicked()
                    {
                        do_copy = true;
                    }
                    if ui.button("取消 (Esc)").clicked() {
                        close_requested = true;
                    }
                });

                let enter = ui.input(|i| i.key_pressed(Key::Enter) && !i.modifiers.shift);
                let all_filled = st.vars.iter().all(|(_, v)| !v.trim().is_empty());
                if enter && all_filled {
                    do_copy = true;
                }
            });

        if do_copy {
            self.fill_params = Some(st);
            self.finish_fill_and_copy();
            return;
        }
        if keep_open && !close_requested {
            self.fill_params = Some(st);
        }
    }

    // ---------- toast ----------

    fn draw_toast(&mut self, ctx: &egui::Context) {
        let Some(t) = &self.toast else { return };
        let color = t.color;
        let msg = t.msg.clone();
        let undoable = self.last_deleted.is_some();
        let mut clicked = false;

        egui::Area::new(egui::Id::new("toast"))
            .fixed_pos(egui::pos2(
                ctx.screen_rect().right() - 260.0,
                ctx.screen_rect().bottom() - 60.0,
            ))
            .show(ctx, |ui| {
                let resp = egui::Frame::popup(ui.style())
                    .fill(color)
                    .show(ui, |ui| {
                        ui.label(RichText::new(msg).color(Color32::WHITE).strong());
                    });
                if undoable {
                    let area = resp.response.interact(egui::Sense::click());
                    if area.clicked() {
                        clicked = true;
                    }
                    area.on_hover_text("点击撤销删除");
                }
            });
        if clicked {
            self.undo_last_delete();
            self.toast = None;
        }
    }

    // ---------- LLM result handler ----------

    fn handle_llm_result(&mut self, r: JobResult) {
        match (r.kind, r.outcome) {
            (JobKind::Enhance, Ok(JobOutput::Enhance(e))) => {
                if let Some(qa) = self.quick_add.as_mut() {
                    // Only apply if the result is for the user's current command —
                    // user may have re-pasted while LLM was in flight.
                    let still_relevant = match (&r.cmd_for_cache, &qa.enhanced_from) {
                        (Some(a), Some(b)) => a == b,
                        _ => true,
                    };
                    if still_relevant {
                        if !qa.title_dirty {
                            if let Some(t) = &e.title {
                                qa.title = t.clone();
                            }
                        }
                        if !qa.tags_dirty && !e.tags.is_empty() {
                            qa.tags = e.tags.clone();
                        }
                        if !qa.desc_dirty {
                            if let Some(d) = &e.description {
                                qa.description = d.clone();
                            }
                        }
                        qa.enhance_status = EnhanceStatus::Done;
                        self.toast_success("🧠 已增强");
                    }
                } else {
                    // Quick add closed — patch the just-saved record if titles still match.
                    if let Some(cmd) = &r.cmd_for_cache {
                        if let Ok(Some(mut note)) = self.db.find_by_exact_command(cmd) {
                            let mut changed = false;
                            if let Some(t) = &e.title {
                                if note.title.is_empty() || note.title == *cmd {
                                    note.title = t.clone();
                                    changed = true;
                                }
                            }
                            if !e.tags.is_empty() && note.tags.is_empty() {
                                note.tags = e.tags.clone();
                                changed = true;
                            }
                            if note.description.trim().is_empty() {
                                if let Some(d) = &e.description {
                                    note.description = d.clone();
                                    changed = true;
                                }
                            }
                            if changed {
                                let _ = self.db.update_command(&note);
                                self.reload();
                                self.toast_success("🧠 已自动补全该命令");
                            }
                        }
                    }
                }
            }
            (JobKind::Enhance, Err(e)) => {
                if let Some(qa) = self.quick_add.as_mut() {
                    qa.enhance_status = EnhanceStatus::Failed;
                }
                self.toast_error(format!("AI 增强失败: {e}"));
            }
            (JobKind::Describe, Ok(JobOutput::Text(t))) => {
                // If the buffer is still on the target command, fill it (becomes dirty, auto-saves)
                let applied_inline = match (&self.edit_buffer, r.target_id) {
                    (Some(b), Some(tid)) if b.command_id == tid => {
                        let buf = self.edit_buffer.as_mut().unwrap();
                        buf.description = t.clone();
                        buf.describing = false;
                        buf.desc_editing = false;
                        buf.dirty = true;
                        buf.last_change_at = Some(Instant::now());
                        true
                    }
                    _ => false,
                };
                if !applied_inline {
                    if let Some(tid) = r.target_id {
                        if let Some(mut c) = self.commands.iter().find(|x| x.id == tid).cloned() {
                            c.description = t;
                            let _ = self.db.update_command(&c);
                            self.reload();
                        }
                    }
                }
                self.toast_success("🧠 已生成说明");
            }
            (JobKind::Describe, Err(e)) => {
                if let Some(buf) = self.edit_buffer.as_mut() {
                    buf.describing = false;
                }
                self.toast_error(format!("AI 生成说明失败: {e}"));
            }
            (JobKind::Generate, Ok(JobOutput::Generate(g))) => {
                let mut qa = QuickAddState::default();
                qa.command = g.command.clone();
                qa.description = g.description;
                qa.desc_dirty = true;
                let r = inference::infer(&qa.command);
                qa.title = r.title;
                qa.category_name = r.category;
                qa.tags = r.tags;
                qa.last_inferred_from = qa.command.clone();
                self.quick_add = Some(qa);
                self.nl_gen = None;
                self.pending_focus.quick_command = true;
                self.toast_success("🧠 已生成命令");
            }
            (JobKind::Generate, Err(e)) => {
                if let Some(ng) = self.nl_gen.as_mut() {
                    ng.in_flight = false;
                }
                self.toast_error(format!("AI 生成命令失败: {e}"));
            }
            (JobKind::Explain, Ok(JobOutput::Text(t))) => {
                if let Some(tid) = r.target_id {
                    if let Some(mut c) = self.commands.iter().find(|x| x.id == tid).cloned() {
                        let sep = if c.description.trim().is_empty() {
                            ""
                        } else {
                            "\n\n---\n\n"
                        };
                        c.description = format!("{}{}## AI 解释\n\n{}", c.description, sep, t);
                        let _ = self.db.update_command(&c);
                        self.reload();
                        self.toast_success("🧠 解释已附到说明");
                    }
                }
            }
            (JobKind::Explain, Err(e)) => {
                self.toast_error(format!("AI 解释失败: {e}"));
            }
            _ => {}
        }
    }

    // ---------- AI: NL generate command dialog ----------

    fn draw_nl_gen(&mut self, ctx: &egui::Context) {
        let Some(mut st) = self.nl_gen.take() else {
            return;
        };
        let mut keep_open = true;
        let mut close_requested = false;
        let mut do_generate = false;

        egui::Window::new("🪄 AI 生成命令")
            .open(&mut keep_open)
            .collapsible(false)
            .resizable(true)
            .default_width(540.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(RichText::new("用自然语言描述，AI 帮你写命令").weak());
                ui.add_space(6.0);
                if !self.llm.is_configured() {
                    ui.label(
                        RichText::new(format!(
                            "⚠ LLM 未配置：{}。在「设置」中配置后再试。",
                            self.llm.missing_reason().unwrap_or_default()
                        ))
                        .color(Color32::from_rgb(200, 140, 60)),
                    );
                }
                ui.add_space(4.0);
                let resp = ui.add(
                    TextEdit::multiline(&mut st.prompt)
                        .hint_text("例：删除 30 天以上的 *.log 文件\n或：查看占用 8080 端口的进程")
                        .desired_width(f32::INFINITY)
                        .desired_rows(4),
                );
                if !st.in_flight {
                    resp.request_focus();
                }
                ui.add_space(6.0);
                if st.in_flight {
                    ui.label(
                        RichText::new("🧠 生成中…")
                            .color(Color32::from_rgb(120, 180, 240)),
                    );
                }
                ui.horizontal(|ui| {
                    let enabled = !st.in_flight
                        && !st.prompt.trim().is_empty()
                        && self.llm.is_configured();
                    if ui
                        .add_enabled(enabled, egui::Button::new(RichText::new("🪄 生成").strong()))
                        .clicked()
                    {
                        do_generate = true;
                    }
                    if ui.button("取消 (Esc)").clicked() {
                        close_requested = true;
                    }
                });
            });

        if do_generate {
            st.in_flight = true;
            self.llm.generate_command(&st.prompt);
        }
        if keep_open && !close_requested {
            self.nl_gen = Some(st);
        }
    }

    // ---------- AI: settings dialog ----------

    fn draw_settings(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let Some(mut draft) = self.settings_draft.take() else {
            self.show_settings = false;
            return;
        };
        let mut keep_open = true;
        let mut close_requested = false;
        let mut do_save = false;
        let mut do_test = false;

        egui::Window::new("⚙ LLM 设置")
            .open(&mut keep_open)
            .collapsible(false)
            .resizable(true)
            .default_width(560.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::Grid::new("settings_grid")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Provider");
                        egui::ComboBox::from_id_salt("settings_provider")
                            .selected_text(draft.provider.label())
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut draft.provider,
                                    Provider::OpenAi,
                                    Provider::OpenAi.label(),
                                );
                                ui.selectable_value(
                                    &mut draft.provider,
                                    Provider::Anthropic,
                                    Provider::Anthropic.label(),
                                );
                            });
                        ui.end_row();

                        ui.label("Base URL");
                        ui.add(
                            TextEdit::singleline(&mut draft.base_url).desired_width(f32::INFINITY),
                        );
                        ui.end_row();

                        ui.label("Model");
                        ui.add(
                            TextEdit::singleline(&mut draft.model).desired_width(f32::INFINITY),
                        );
                        ui.end_row();

                        ui.label("API key 环境变量");
                        ui.add(
                            TextEdit::singleline(&mut draft.api_key_env)
                                .hint_text("OPENAI_API_KEY / ANTHROPIC_API_KEY ...")
                                .desired_width(f32::INFINITY),
                        );
                        ui.end_row();

                        // API key value — Keychain-backed (with env-var override visible)
                        ui.label("API key");
                        ui.vertical(|ui| {
                            let source = self.llm.api_key_source();
                            let (label_txt, label_color) = match source {
                                crate::llm::ApiKeySource::Env => (
                                    format!("从环境变量 ${} 读取（优先于 Keychain）", draft.api_key_env),
                                    Color32::from_rgb(120, 180, 240),
                                ),
                                crate::llm::ApiKeySource::Keychain => (
                                    "已保存到系统 Keychain".to_string(),
                                    Color32::from_rgb(140, 200, 140),
                                ),
                                crate::llm::ApiKeySource::None => (
                                    "未配置".to_string(),
                                    Color32::from_rgb(200, 140, 60),
                                ),
                            };
                            ui.label(RichText::new(label_txt).small().color(label_color));
                            ui.horizontal(|ui| {
                                let r = ui.add(
                                    TextEdit::singleline(&mut self.settings_api_key_input)
                                        .password(!self.settings_api_key_visible)
                                        .hint_text("sk-... (粘贴 key 然后点保存)")
                                        .desired_width(280.0),
                                );
                                let _ = r;
                                let eye = if self.settings_api_key_visible { "🙈" } else { "👁" };
                                if ui.small_button(eye).on_hover_text("显示/隐藏").clicked() {
                                    self.settings_api_key_visible = !self.settings_api_key_visible;
                                }
                            });
                            ui.horizontal(|ui| {
                                let can_save = !self.settings_api_key_input.trim().is_empty();
                                if ui
                                    .add_enabled(can_save, egui::Button::new("💾 存到 Keychain"))
                                    .clicked()
                                {
                                    let env_name = draft.api_key_env.clone();
                                    let key = self.settings_api_key_input.trim().to_string();
                                    match crate::llm::keyring_set(&env_name, &key) {
                                        Ok(_) => {
                                            self.toast_success("已存入 Keychain");
                                            self.settings_api_key_input.clear();
                                            self.llm.refresh_api_key_cache();
                                        }
                                        Err(e) => self.toast_error(format!("存 Keychain 失败: {e}")),
                                    }
                                }
                                if source == crate::llm::ApiKeySource::Keychain {
                                    if ui
                                        .button(RichText::new("🗑 删除").color(Color32::from_rgb(200, 80, 80)))
                                        .clicked()
                                    {
                                        match crate::llm::keyring_delete(&draft.api_key_env) {
                                            Ok(_) => {
                                                self.toast_success("已从 Keychain 删除");
                                                self.llm.refresh_api_key_cache();
                                            }
                                            Err(e) => self.toast_error(format!("删除失败: {e}")),
                                        }
                                    }
                                }
                            });
                        });
                        ui.end_row();

                        ui.label("超时（秒）");
                        ui.add(egui::DragValue::new(&mut draft.timeout_secs).range(3..=60));
                        ui.end_row();

                        ui.label("Max tokens");
                        ui.add(egui::DragValue::new(&mut draft.max_tokens).range(64..=4096));
                        ui.end_row();

                        ui.label("速率上限/分钟");
                        ui.add(egui::DragValue::new(&mut draft.rate_per_minute).range(1..=240));
                        ui.end_row();
                    });

                ui.add_space(6.0);
                ui.separator();
                ui.add_space(4.0);
                ui.label(RichText::new("功能开关").strong());
                ui.add_space(2.0);

                // Paste enhance — needs consent on first enable
                let was = draft.features.paste_enhance;
                ui.checkbox(&mut draft.features.paste_enhance, "🧠 粘贴即增强（后台 AI）");
                if !was && draft.features.paste_enhance {
                    if draft.consent.paste_enhance.is_none() {
                        self.pending_consent = Some(ConsentSubject::PasteEnhance);
                    }
                }

                ui.checkbox(&mut draft.features.generate_description, "✨ 一键生成 description");
                ui.checkbox(&mut draft.features.generate_command, "🪄 自然语言生成命令");
                ui.checkbox(&mut draft.features.explain_command, "🧠 解释命令");
                ui.add_space(4.0);
                ui.checkbox(&mut draft.offline_mode, "🚫 离线模式（完全禁用远程调用）");

                if let Some(reason) = self.llm.missing_reason() {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(format!("⚠ {reason}"))
                            .color(Color32::from_rgb(200, 140, 60)),
                    );
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("🔌 测试连接").clicked() {
                        do_test = true;
                    }
                    if let Some(m) = &self.settings_test_msg {
                        ui.label(m);
                    }
                });

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("保存").strong()).clicked() {
                        do_save = true;
                    }
                    if ui.button("取消 (Esc)").clicked() {
                        close_requested = true;
                    }
                });

                ui.add_space(6.0);
                ui.label(
                    RichText::new("📁 配置写入: ~/.config/Spellbook/llm.toml（API key 仅读自环境变量）")
                        .weak()
                        .small(),
                );
            });

        if do_test {
            // do a sync test using the draft config
            let prev = self.llm.config.clone();
            self.llm.config = draft.clone();
            self.settings_test_msg = match self.llm.test_connection() {
                Ok(s) => Some(format!("✓ 收到回复: {}", s.chars().take(40).collect::<String>())),
                Err(e) => Some(format!("✗ {e}")),
            };
            self.llm.config = prev;
        }

        if do_save {
            self.llm.config = draft.clone();
            if let Err(e) = self.llm.save_config() {
                self.toast_error(format!("保存配置失败: {e}"));
            } else {
                self.toast_success("已保存");
            }
            self.show_settings = false;
            self.settings_draft = None;
            self.settings_test_msg = None;
            self.settings_api_key_input.clear();
            self.settings_api_key_visible = false;
            return;
        }

        if !keep_open || close_requested {
            self.show_settings = false;
            self.settings_draft = None;
            self.settings_test_msg = None;
            self.settings_api_key_input.clear();
            self.settings_api_key_visible = false;
        } else {
            self.settings_draft = Some(draft);
        }
    }

    // ---------- AI: consent dialog ----------

    fn draw_consent(&mut self, ctx: &egui::Context) {
        let Some(subject) = self.pending_consent else {
            return;
        };
        let mut keep_open = true;
        let mut confirm = false;
        let mut cancel = false;

        let provider_label = self
            .settings_draft
            .as_ref()
            .map(|d| d.provider.label())
            .unwrap_or_else(|| self.llm.config.provider.label());
        let base = self
            .settings_draft
            .as_ref()
            .map(|d| d.base_url.clone())
            .unwrap_or_else(|| self.llm.config.base_url.clone());

        egui::Window::new("🧠 启用 \"粘贴即增强\"")
            .open(&mut keep_open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("启用后，每次粘贴新命令时会自动把以下内容发送给：");
                ui.add_space(4.0);
                ui.label(RichText::new(format!("  {provider_label}  ·  {base}")).monospace());
                ui.add_space(8.0);
                ui.label("• 命令文本（command）");
                ui.label("• 规则推断出的初始 title / category / tags");
                ui.add_space(6.0);
                ui.label(RichText::new("不会发送：").strong());
                ui.label("• 你的其他命令");
                ui.label("• 任何个人信息");
                ui.label("• API key（始终只从环境变量读取）");
                ui.add_space(6.0);
                ui.label(RichText::new("可随时在设置里关闭。").weak());
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("我了解，启用").strong()).clicked() {
                        confirm = true;
                    }
                    if ui.button("取消").clicked() {
                        cancel = true;
                    }
                });
            });

        if confirm {
            match subject {
                ConsentSubject::PasteEnhance => {
                    if let Some(draft) = self.settings_draft.as_mut() {
                        draft.consent.paste_enhance = Some(chrono::Utc::now());
                    } else {
                        self.llm.config.consent.paste_enhance = Some(chrono::Utc::now());
                        let _ = self.llm.save_config();
                    }
                }
            }
            self.pending_consent = None;
        } else if cancel || !keep_open {
            // revert the checkbox in the draft
            if let Some(draft) = self.settings_draft.as_mut() {
                match subject {
                    ConsentSubject::PasteEnhance => draft.features.paste_enhance = false,
                }
            }
            self.pending_consent = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_variables_basic() {
        let v = parse_variables("ssh ${USER}@${HOST}");
        assert_eq!(v, vec!["USER", "HOST"]);
    }

    #[test]
    fn parse_variables_dedupes() {
        let v = parse_variables("scp ${HOST}:a ${HOST}:b");
        assert_eq!(v, vec!["HOST"]);
    }

    #[test]
    fn parse_variables_ignores_plain_dollar() {
        let v = parse_variables("echo $HOME ${REAL}");
        assert_eq!(v, vec!["REAL"]);
    }

    #[test]
    fn parse_variables_skips_unclosed() {
        let v = parse_variables("echo ${broken");
        assert!(v.is_empty());
    }

    #[test]
    fn parse_variables_rejects_invalid_chars() {
        // spaces and special chars inside braces → reject
        let v = parse_variables("echo ${has space} ${ok_name}");
        assert_eq!(v, vec!["ok_name"]);
    }

    #[test]
    fn substitute_basic() {
        let s = substitute_variables(
            "ssh ${USER}@${HOST}",
            &[("USER".into(), "alice".into()), ("HOST".into(), "h1".into())],
        );
        assert_eq!(s, "ssh alice@h1");
    }

    #[test]
    fn substitute_with_empty_value_leaves_placeholder_blanked() {
        let s = substitute_variables(
            "echo ${A} ${B}",
            &[("A".into(), "x".into()), ("B".into(), "".into())],
        );
        assert_eq!(s, "echo x ");
    }
}

/// Extract `${VAR}` placeholders from a command, preserving first-seen order
/// and deduping by name. `$NAME` (no braces) is intentionally NOT parsed so
/// shell variables like `$HOME` aren't accidentally treated as placeholders.
pub fn parse_variables(command: &str) -> Vec<String> {
    let mut vars = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let bytes = command.as_bytes();
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == b'$' && bytes[i + 1] == b'{' {
            let mut j = i + 2;
            while j < bytes.len() && bytes[j] != b'}' {
                j += 1;
            }
            if j < bytes.len() {
                if let Ok(name) = std::str::from_utf8(&bytes[i + 2..j]) {
                    let name = name.trim().to_string();
                    if !name.is_empty()
                        && name.chars().all(|c| {
                            c.is_ascii_alphanumeric() || c == '_' || c == '-'
                        })
                        && seen.insert(name.clone())
                    {
                        vars.push(name);
                    }
                }
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    vars
}

pub fn substitute_variables(template: &str, vars: &[(String, String)]) -> String {
    let mut s = template.to_string();
    for (name, value) in vars {
        let needle = format!("${{{}}}", name);
        s = s.replace(&needle, value);
    }
    s
}

fn open_url(url: &str) -> std::io::Result<()> {
    let cmd = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    std::process::Command::new(cmd).arg(url).spawn().map(|_| ())
}

fn first_line(s: &str, max: usize) -> String {
    let line = s.lines().next().unwrap_or("").trim();
    if line.chars().count() <= max {
        line.to_string()
    } else {
        let trunc: String = line.chars().take(max - 1).collect();
        format!("{}…", trunc)
    }
}

// ============================================================================
// Tag chip — colored, rounded, hash-stable palette.
// ============================================================================

pub struct TagChipResponse {
    pub response: egui::Response,
    pub remove_clicked: bool,
}

/// 8 tasteful (bg, fg) pairs that read well on a dark background.
const TAG_PALETTE: &[(Color32, Color32)] = &[
    (Color32::from_rgb(40, 65, 100),  Color32::from_rgb(170, 210, 255)),  // blue
    (Color32::from_rgb(40, 70, 50),   Color32::from_rgb(160, 220, 170)),  // green
    (Color32::from_rgb(85, 55, 35),   Color32::from_rgb(255, 195, 140)),  // orange
    (Color32::from_rgb(75, 50, 90),   Color32::from_rgb(220, 175, 245)),  // purple
    (Color32::from_rgb(85, 75, 30),   Color32::from_rgb(240, 220, 110)),  // yellow
    (Color32::from_rgb(85, 35, 60),   Color32::from_rgb(245, 160, 200)),  // pink
    (Color32::from_rgb(30, 75, 85),   Color32::from_rgb(150, 230, 235)),  // cyan
    (Color32::from_rgb(70, 70, 75),   Color32::from_rgb(200, 200, 210)),  // neutral
];

fn tag_palette(tag: &str) -> (Color32, Color32) {
    // djb2 hash → palette index. Same tag always picks the same color
    // across sessions and views.
    let mut h: u32 = 5381;
    for b in tag.bytes() {
        h = h.wrapping_mul(33).wrapping_add(b as u32);
    }
    TAG_PALETTE[(h as usize) % TAG_PALETTE.len()]
}

/// Render one tag as a pill. If `removable`, shows an `×` that the caller can
/// poll via `remove_clicked`. The whole chip is clickable (e.g. to filter).
fn tag_chip(ui: &mut Ui, tag: &str, removable: bool, compact: bool) -> TagChipResponse {
    let (bg, fg) = tag_palette(tag);
    let pad_x = if compact { 6.0 } else { 8.0 };
    let pad_y = if compact { 1.0 } else { 2.0 };

    let frame = egui::Frame::none()
        .fill(bg)
        .rounding(egui::Rounding::same(10.0))
        .inner_margin(egui::Margin::symmetric(pad_x, pad_y));

    let inner = frame.show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.horizontal(|ui| {
            let text = if compact {
                RichText::new(tag).small().color(fg)
            } else {
                RichText::new(tag).color(fg).small()
            };
            ui.label(text);
            let mut remove_clicked = false;
            if removable {
                let x = ui.add(
                    egui::Button::new(RichText::new("×").color(fg))
                        .small()
                        .frame(false),
                );
                if x.clicked() {
                    remove_clicked = true;
                }
            }
            remove_clicked
        })
        .inner
    });

    let response = inner.response.interact(egui::Sense::click());
    TagChipResponse {
        response,
        remove_clicked: inner.inner,
    }
}
