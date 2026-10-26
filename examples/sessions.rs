//! Diagnostik: daftar semua sesi media yang dilihat Windows (SMTC).
//! Jalankan: cargo run --example sessions

use windows::Media::Control::*;

fn main() -> windows::core::Result<()> {
    let mgr = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?.get()?;
    let cur = mgr.GetCurrentSession().ok().and_then(|s| s.SourceAppUserModelId().ok());
    println!("current: {:?}", cur.map(|s| s.to_string_lossy()));
    for (i, s) in mgr.GetSessions()?.into_iter().enumerate() {
        if let Some(path) = std::env::args().nth(1) {
            save_thumb(&s, &format!("{path}-{i}.img"));
        }
        let app = s.SourceAppUserModelId()?.to_string_lossy();
        let status = s.GetPlaybackInfo()?.PlaybackStatus()?.0;
        let p = s.TryGetMediaPropertiesAsync()?.get()?;
        let thumb = p
            .Thumbnail()
            .ok()
            .and_then(|t| t.OpenReadAsync().ok()?.get().ok())
            .and_then(|st| Some((st.Size().ok()?, st.ContentType().ok()?.to_string_lossy())));
        println!(
            "- app={app} status={status} (4=playing,5=paused) title={:?} artist={:?} album={:?} thumb={:?}",
            p.Title()?.to_string_lossy(),
            p.Artist()?.to_string_lossy(),
            p.AlbumTitle()?.to_string_lossy(),
            thumb
        );
    }
    Ok(())
}

/// Simpan sampul sesi ke file, untuk dilihat manual.
fn save_thumb(s: &GlobalSystemMediaTransportControlsSession, path: &str) -> Option<()> {
    use windows::Storage::Streams::DataReader;
    let p = s.TryGetMediaPropertiesAsync().ok()?.get().ok()?;
    let st = p.Thumbnail().ok()?.OpenReadAsync().ok()?.get().ok()?;
    let n = st.Size().ok()? as u32;
    let r = DataReader::CreateDataReader(&st.GetInputStreamAt(0).ok()?).ok()?;
    r.LoadAsync(n).ok()?.get().ok()?;
    let mut buf = vec![0u8; n as usize];
    r.ReadBytes(&mut buf).ok()?;
    std::fs::write(path, buf).ok()
}
