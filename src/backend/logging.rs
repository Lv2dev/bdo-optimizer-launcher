// M63: 파일 기반 진단 로그. windows_subsystem="windows"에서 eprintln!이 사라지므로
// 사용자 보고를 받을 유일한 채널이다.
//
// 위치: %LOCALAPPDATA%\bdo-optimizer-launcher\logs\bdo-optimizer.YYYY-MM-DD
// 회전: 일일 (tracing-appender::rolling::daily).
// 레벨: 기본 INFO, `RUST_LOG` env로 override 가능 (`RUST_LOG=debug` 등).
// panic hook: 패닉 발생 시 로그에 backtrace 기록 후 기본 hook 호출.

use std::path::PathBuf;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

const LOG_FILE_PREFIX: &str = "bdo-optimizer";

// %LOCALAPPDATA%\bdo-optimizer-launcher\logs\.
// LOCALAPPDATA 부재 시 None — 로그 비활성(설치 폴더 오염 회피, settings.rs M24 패턴).
pub fn log_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|root| {
        PathBuf::from(root)
            .join("bdo-optimizer-launcher")
            .join("logs")
    })
}

// M66b: thiserror enum.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("LOCALAPPDATA 환경변수를 찾을 수 없습니다.")]
    NoLocalAppData,
    #[error("로그 폴더 생성 실패: {0}")]
    CreateDir(std::path::PathBuf),
    #[error("explorer 실행 실패: {0}")]
    ExplorerSpawn(#[from] std::io::Error),
}

/// 사용자에게 로그 폴더를 explorer.exe로 표시. 폴더가 없으면 생성 후 열기.
/// `system_command`를 쓰지 않음 — explorer.exe는 system32가 아닌 Windows 루트에 있고,
/// CREATE_NO_WINDOW를 explorer에 적용하면 정상 동작 안 함.
pub fn open_log_folder() -> Result<(), Error> {
    let dir = log_dir().ok_or(Error::NoLocalAppData)?;
    if std::fs::create_dir_all(&dir).is_err() {
        return Err(Error::CreateDir(dir));
    }
    std::process::Command::new(super::windows_path("explorer.exe"))
        .arg(&dir)
        .spawn()
        .map(|_| ())
        .map_err(Error::ExplorerSpawn)
}

/// 파일 로거 초기화. 반환된 `WorkerGuard`를 main 라이프타임에 유지해야 비동기 writer가 flush 보장.
/// 실패 시(LOCALAPPDATA 부재 / 디렉터리 생성 실패) None 반환 — 앱 동작에 영향 없음.
pub fn init() -> Option<WorkerGuard> {
    let dir = log_dir()?;
    if std::fs::create_dir_all(&dir).is_err() {
        return None;
    }

    let file_appender = tracing_appender::rolling::daily(&dir, LOG_FILE_PREFIX);
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let subscriber = tracing_subscriber::fmt()
        .with_writer(non_blocking)
        .with_env_filter(filter)
        .with_ansi(false)
        .with_target(true)
        .with_level(true)
        .with_thread_ids(false)
        .with_thread_names(false)
        .compact();

    if subscriber.try_init().is_err() {
        // 이미 다른 subscriber가 등록된 경우 — 정상 (테스트 환경 등).
        return Some(guard);
    }

    register_panic_hook();
    tracing::info!(
        log_dir = %dir.display(),
        version = env!("CARGO_PKG_VERSION"),
        "logger initialized"
    );
    Some(guard)
}

// 패닉 발생 시 location + message를 ERROR로 로깅. 기존 hook도 호출(콘솔 출력 등 표준 동작 보존).
fn register_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown>".to_string());
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(|s| s.as_str()))
            .unwrap_or("<non-string panic payload>");
        tracing::error!(location = %location, message = %payload, "panic");
        prev(info);
    }));
}
