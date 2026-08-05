mod archive;
mod qlogin;
mod qzone;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(archive::ArchiveState::new())
        .manage(qlogin::QLoginState::new())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            qlogin::start_qr_login,
            qlogin::poll_qr_login,
            qlogin::cancel_qr_login,
            qlogin::get_login_status,
            qlogin::get_qzone_login_user,
            qlogin::logout_qzone,
            qlogin::open_web_login,
            qlogin::check_web_login,
            qlogin::cancel_web_login,
            qzone::fetch_first_feeds,
            qzone::fetch_more_feeds,
            archive::start_feed_archive,
            archive::get_archive_progress,
            archive::cancel_feed_archive,
            archive::list_archive_skips,
            archive::retry_archive_skip,
            archive::list_archived_feeds,
            archive::list_archived_media,
            archive::get_archived_feed,
            archive::count_archived_feeds,
            archive::export_archived_html,
            archive::load_archived_image,
            archive::load_archived_video,
            archive::get_archive_overview,
            archive::list_interactors,
            archive::get_interaction_ranking,
            archive::delete_archived_feeds,
            archive::clear_archived_feeds,
            archive::delete_all_app_data,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
