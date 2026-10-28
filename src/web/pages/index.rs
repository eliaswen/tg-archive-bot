use super::*;

pub(in crate::web) async fn index(State(data): State<WebData>) -> WebResult {
    let stats = crate::archive_stats::load(&data.pool)
        .await
        .map_err(database_error)?;
    let body = render_template(&IndexTemplate {
        message_count: stats.messages,
        user_count: stats.users,
        server_count: stats.servers,
        channel_count: stats.channels,
        total_storage: format_bytes(stats.total_storage),
        message_storage: format_bytes(stats.message_storage),
        attachment_storage: format_bytes(stats.attachment_storage),
        processed_images: stats.processed_images,
        unprocessed_images: stats.unprocessed_images,
        average_image_processing_speed: crate::archive_stats::format_processing_speed(
            stats.average_image_processing_ms,
        ),
        average_daily_storage: format_bytes(stats.average_daily_storage),
        running_time: crate::archive_stats::format_duration(stats.running_seconds),
    });
    Ok(Html(page("Archive", &body)))
}
