//! What Copy diagnostics puts on the clipboard, for a bug report.
//!
//! **Nothing in it can sign anyone in or name anyone's chats.** The settings
//! go through `Settings::without_credentials`, which drops the api_id, the
//! api_hash, the phone number and the saved selections — the run record in
//! every database is written the same way, for the same reason. What is left
//! is the version, the system, where the program and its data live, and how
//! it is set up. The paths can carry a Windows user name, which is why the
//! button's tooltip says to look the text over before posting it.

use tgx_tg::config::Settings;

pub fn report(settings: &Settings) -> String {
    let mut settings_json = settings.without_credentials();
    // Where the window was is noise in a bug report.
    if let Some(map) = settings_json.as_object_mut() {
        map.remove("window");
        map.remove("window_maximized");
    }
    let settings_text =
        serde_json::to_string_pretty(&settings_json).unwrap_or_else(|_| "{}".into());
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "unknown".into());
    let protected = match tgx_tg::config::lockdown_error() {
        None => "yes".to_string(),
        Some(why) => format!("NO — {why}"),
    };
    format!(
        "TelegramTools {version}\n\
         System: {os}\n\
         Program: {exe}\n\
         Data folder: {data}\n\
         Data folder restricted to this user: {protected}\n\
         Signed-in session on disk: {session}\n\
         Settings (credentials and saved selections removed):\n\
         {settings_text}\n",
        version = env!("CARGO_PKG_VERSION"),
        os = os_info::get(),
        data = tgx_tg::config::data_dir().display(),
        session = if tgx_tg::config::session_file().exists() {
            "yes"
        } else {
            "no"
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_credential_or_selection_reaches_the_clipboard() {
        let settings = Settings {
            api_id: 123_456_789,
            api_hash: "0123456789abcdef0123456789abcdef".into(),
            phone: "+15555550123".into(),
            saved_selections: vec![tgx_tg::config::SavedSelection {
                name: "Private family chats".into(),
                chats: vec![42],
            }],
            ..Settings::default()
        };
        let text = report(&settings);
        for secret in [
            "123456789",
            "0123456789abcdef",
            "+15555550123",
            "Private family chats",
        ] {
            assert!(!text.contains(secret), "{secret} leaked into: {text}");
        }
        assert!(text.contains(env!("CARGO_PKG_VERSION")));
        assert!(text.contains("\"page_size\""));
    }
}
