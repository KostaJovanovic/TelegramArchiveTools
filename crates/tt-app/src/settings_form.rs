//! The editable settings, and the rules for reading a number out of a field.
//!
//! **Nothing here coerces.** `Settings::load_from_str` already refuses to read
//! `"20"` as `20` — *"a guess would be worse than a default"* — and a text
//! field is the same problem arriving by a different route. A field holding
//! nonsense keeps the stored value rather than becoming zero, and a field
//! holding a number outside its range is clamped **and written back**, so what
//! is on screen is what is stored.
//!
//! **The fields are `String`s now.** Under GPUI each was an
//! `Entity<InputState>` that could only be built with a live `Window`, which is
//! why the shell carried `form: Option<SettingsForm>` and documented it as
//! "`None` in the headless shell the interaction tests drive". egui binds a text
//! field to a `&mut String`, so the buffer is ordinary data, the `Option` is
//! gone, and the tests reach the same fields the window does.

use tgx_tg::config::Settings;

/// Messages per `messages*.html` page. Desktop's own default is 1,000.
pub const PAGE_RANGE: (i64, i64) = (50, 20_000);
/// Parallel media downloads. 4-6 is a good balance; above that Telegram starts
/// rate-limiting the account rather than going faster.
pub const DOWNLOAD_RANGE: (i64, i64) = (1, 16);
/// Size limit in MB. `0` is unlimited, which is why the floor is zero and not
/// one — see [`Settings::size_limit_bytes`].
pub const SIZE_RANGE: (i64, i64) = (0, 20_000);
/// Member roster cap. `0` is no cap. A public channel can have millions of
/// members and Telegram stops serving the listing long before that.
pub const MEMBER_RANGE: (i64, i64) = (0, 1_000_000);
/// How many of the newest archived messages a database-only sync re-reads. `0`
/// is a real answer — "only what is new" — so the floor is zero. The ceiling is
/// where a sync stops being faster than the whole-history pass it replaces.
pub const REREAD_RANGE: (i64, i64) = (0, 10_000);
/// Hours between scheduled syncs. `0` is off; the ceiling is a week.
pub const SYNC_RANGE: (i64, i64) = (0, 168);
/// The model server's port. Below 1024 is the system's.
pub const PORT_RANGE: (i64, i64) = (1024, 65_535);
/// Minutes before an idle model server started from here is stopped. At least
/// one, so a summary and the next are not each a fresh load; at most a day.
pub const IDLE_RANGE: (i64, i64) = (1, 1_440);

/// Read a number out of a text field.
///
/// Returns `current` for anything that is not a whole number, including an
/// empty field: a half-typed value must not be read as a decision. Everything
/// else is clamped into range, and the caller writes the result back so the
/// field cannot disagree with what was stored.
pub fn number(raw: &str, range: (i64, i64), current: i64) -> i64 {
    let trimmed = raw.trim();
    match trimmed.parse::<i64>() {
        Ok(n) => n.clamp(range.0, range.1),
        Err(_) => current,
    }
}

/// The text fields the settings panel owns.
///
/// Checkboxes are not here: a tick is a `bool` on [`Settings`] and needs no
/// buffer between the click and the value, so giving it one would only create
/// a second place for the answer to live.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SettingsForm {
    pub output_dir: String,
    pub page_size: String,
    pub size_limit: String,
    pub downloads: String,
    pub member_limit: String,
    pub reread_window: String,
    pub sync_every: String,
    /// `YYYY-MM-DD` or blank. Kept as typed: a date is not clamped into
    /// anything, and one that does not parse is refused when a run starts —
    /// see `Settings::date_range` — with the panel saying so under the field.
    pub date_from: String,
    pub date_until: String,
    /// Settings › Summaries: the models folder, the server, the port, the
    /// extra options and the idle minutes. The model is a dropdown, which
    /// writes the setting itself.
    pub summary_models: String,
    pub summary_server: String,
    pub summary_port: String,
    pub summary_options: String,
    pub summary_idle: String,
}

impl SettingsForm {
    pub fn new(settings: &Settings) -> Self {
        let mut form = Self::default();
        form.sync(settings);
        form
    }

    /// Read every field back into `settings`.
    ///
    /// **The destination keeps its stored value when the field is blank.** An
    /// export into `""` resolves to the process's working directory, which for
    /// a portable exe is wherever the shell happened to be — a folder the user
    /// never chose and will not think to look in.
    pub fn collect(&self, settings: &mut Settings) {
        let dir = self.output_dir.trim().to_string();
        if !dir.is_empty() {
            settings.output_dir = dir;
        }
        settings.page_size =
            number(&self.page_size, PAGE_RANGE, settings.page_size as i64) as usize;
        settings.size_limit_mb = number(&self.size_limit, SIZE_RANGE, settings.size_limit_mb);
        settings.download_concurrency = number(
            &self.downloads,
            DOWNLOAD_RANGE,
            settings.download_concurrency as i64,
        ) as usize;
        settings.member_limit = number(&self.member_limit, MEMBER_RANGE, settings.member_limit);
        settings.reread_window = number(
            &self.reread_window,
            REREAD_RANGE,
            settings.reread_window as i64,
        ) as usize;
        settings.sync_every_hours = number(
            &self.sync_every,
            SYNC_RANGE,
            i64::from(settings.sync_every_hours),
        ) as u32;
        settings.date_from = self.date_from.trim().to_string();
        settings.date_until = self.date_until.trim().to_string();
        // Paths as typed, quotes from Explorer's "Copy as path" taken off. A
        // blank models folder is the default one, said in the field.
        let models = self.summary_models.trim().trim_matches('"');
        settings.summary_models = if models.is_empty() {
            Settings::default().summary_models
        } else {
            models.to_string()
        };
        settings.summary_server = self.summary_server.trim().trim_matches('"').to_string();
        settings.summary_port = number(
            &self.summary_port,
            PORT_RANGE,
            i64::from(settings.summary_port),
        ) as u16;
        settings.summary_options = self.summary_options.trim().to_string();
        settings.summary_idle_minutes = number(
            &self.summary_idle,
            IDLE_RANGE,
            i64::from(settings.summary_idle_minutes),
        ) as u32;
    }

    /// Write the stored values back into the fields.
    ///
    /// Run after [`collect`](Self::collect), so a clamped or rejected entry is
    /// *visible*. Without it someone types `0` into Messages per page, the
    /// setting becomes 50, and the field goes on reading `0` — the interface
    /// asserting something about the export that is not true.
    ///
    /// Each field is compared before it is written. Under GPUI that mattered
    /// because assigning an identical value still moved the caret to the end,
    /// turning a mid-path edit into a fight with the field; egui keeps the caret
    /// in its own state, but the guard is kept because the same reasoning
    /// applies the moment anything watches these for changes.
    pub fn sync(&mut self, settings: &Settings) {
        set(&mut self.output_dir, &settings.output_dir);
        set(&mut self.page_size, &settings.page_size.to_string());
        set(&mut self.size_limit, &settings.size_limit_mb.to_string());
        set(
            &mut self.downloads,
            &settings.download_concurrency.to_string(),
        );
        set(&mut self.member_limit, &settings.member_limit.to_string());
        set(&mut self.reread_window, &settings.reread_window.to_string());
        set(&mut self.sync_every, &settings.sync_every_hours.to_string());
        set(&mut self.date_from, &settings.date_from);
        set(&mut self.date_until, &settings.date_until);
        set(&mut self.summary_models, &settings.summary_models);
        set(&mut self.summary_server, &settings.summary_server);
        set(&mut self.summary_port, &settings.summary_port.to_string());
        set(&mut self.summary_options, &settings.summary_options);
        set(
            &mut self.summary_idle,
            &settings.summary_idle_minutes.to_string(),
        );
    }
}

fn set(field: &mut String, value: &str) {
    if field != value {
        field.clear();
        field.push_str(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonsense_keeps_the_stored_value_rather_than_becoming_zero() {
        // The same refusal `Settings::load_from_str` makes: a guess would be
        // worse than what is already there.
        assert_eq!(number("twenty", PAGE_RANGE, 1000), 1000);
        assert_eq!(number("", PAGE_RANGE, 1000), 1000);
        assert_eq!(number("  ", PAGE_RANGE, 1000), 1000);
        assert_eq!(number("12.5", PAGE_RANGE, 1000), 1000);
    }

    #[test]
    fn a_number_out_of_range_is_clamped_not_rejected() {
        // Rejecting it would leave the field and the setting disagreeing with
        // no way for the user to tell which won.
        assert_eq!(number("0", PAGE_RANGE, 1000), 50);
        assert_eq!(number("999999", PAGE_RANGE, 1000), 20_000);
        assert_eq!(number("-4", DOWNLOAD_RANGE, 5), 1);
        assert_eq!(number("500", PAGE_RANGE, 1000), 500);
    }

    #[test]
    fn zero_is_a_legitimate_size_limit_and_member_cap() {
        // 0 means unlimited in both, so the floor is zero — clamping it up to
        // one would quietly cap every download at a megabyte.
        assert_eq!(number("0", SIZE_RANGE, 20), 0);
        assert_eq!(number("0", MEMBER_RANGE, 10_000), 0);
    }

    #[test]
    fn a_clamped_entry_is_written_back_into_its_field() {
        // The whole reason `sync` runs after `collect`: without it someone
        // types 0 into Messages per page, the setting becomes 50, and the field
        // goes on reading 0.
        let mut settings = Settings::default();
        let mut form = SettingsForm::new(&settings);
        form.page_size = "0".into();
        form.collect(&mut settings);
        form.sync(&settings);
        assert_eq!(settings.page_size, 50);
        assert_eq!(form.page_size, "50");
    }

    #[test]
    fn a_blank_destination_keeps_the_stored_one() {
        // An export into "" lands in the process's working directory, which for
        // a portable exe is wherever the shell happened to be.
        let mut settings = Settings::default();
        let stored = settings.output_dir.clone();
        let mut form = SettingsForm::new(&settings);
        form.output_dir = "   ".into();
        form.collect(&mut settings);
        assert_eq!(settings.output_dir, stored);
    }

    #[test]
    fn a_fresh_form_already_holds_what_is_stored() {
        // It is built from the settings rather than from blanks, so the panel
        // does not open on empty fields over a configured export.
        let settings = Settings::default();
        let form = SettingsForm::new(&settings);
        assert_eq!(form.page_size, settings.page_size.to_string());
        assert_eq!(form.output_dir, settings.output_dir);
    }
}
