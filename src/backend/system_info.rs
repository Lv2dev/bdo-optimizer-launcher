// PC 사양(CPU 이름 + GPU 이름) 정적 조회.
// - CPU: Windows Registry `HKLM\HARDWARE\DESCRIPTION\System\CentralProcessor\0`의
//   `ProcessorNameString` 값을 `reg query`로 추출 (settings.rs OS 테마 감지와 동일 패턴).
// - GPU: DXGI `EnumAdapters1` + `DXGI_ADAPTER_FLAG_SOFTWARE` 필터 (monitor.rs M30과 동일 패턴).
// 앱 시작 시 1회만 호출(`monitor_ui::apply_initial`). 부품 핫플러그 시 재시작 필요.

#[derive(Debug, Clone)]
pub struct SystemInfo {
    pub cpu_name: String,
    pub gpu_names: Vec<String>,
}

pub fn fetch_system_info() -> SystemInfo {
    SystemInfo {
        cpu_name: query_cpu_name().unwrap_or_else(|| "Unknown CPU".to_string()),
        gpu_names: query_gpu_names(),
    }
}

#[cfg(windows)]
fn query_cpu_name() -> Option<String> {
    let out = super::system_command("reg.exe")
        .args([
            "query",
            r"HKLM\HARDWARE\DESCRIPTION\System\CentralProcessor\0",
            "/v",
            "ProcessorNameString",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_reg_sz_value(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(not(windows))]
fn query_cpu_name() -> Option<String> {
    None
}

#[cfg(windows)]
fn query_gpu_names() -> Vec<String> {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1};
    let mut names: Vec<String> = Vec::new();
    let factory: IDXGIFactory1 = match unsafe { CreateDXGIFactory1() } {
        Ok(f) => f,
        Err(_) => return names,
    };
    let mut idx = 0u32;
    loop {
        let adapter: IDXGIAdapter1 = match unsafe { factory.EnumAdapters1(idx) } {
            Ok(a) => a,
            Err(_) => break,
        };
        if let Ok(desc) = unsafe { adapter.GetDesc1() } {
            if !is_software_adapter(&desc) {
                let name = decode_description(&desc.Description);
                // 같은 물리 GPU가 여러 어댑터 슬롯으로 열거되는 환경(M47 사용자 보고)
                // 대응. 첫 등장 순서를 유지하며 이름 기준 중복 제거.
                if !name.is_empty() && !names.contains(&name) {
                    names.push(name);
                }
            }
        }
        idx += 1;
    }
    names
}

#[cfg(not(windows))]
fn query_gpu_names() -> Vec<String> {
    Vec::new()
}

#[cfg(windows)]
fn is_software_adapter(desc: &windows::Win32::Graphics::Dxgi::DXGI_ADAPTER_DESC1) -> bool {
    use windows::Win32::Graphics::Dxgi::DXGI_ADAPTER_FLAG_SOFTWARE;
    desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0
}

/// `reg query ... /v ProcessorNameString` 출력에서 REG_SZ 값을 추출.
/// 표준 출력 예 (settings.rs detect_os_dark_mode와 동일 패턴):
///   `HKLM\HARDWARE\...`
///   `    ProcessorNameString    REG_SZ    Intel(R) Core(TM) i7-13700K`
/// 첫 REG_SZ 라인의 값(트림) 반환. 값 부분이 비어 있거나 없으면 None.
fn parse_reg_sz_value(text: &str) -> Option<String> {
    for line in text.lines() {
        if let Some((_, val)) = line.split_once("REG_SZ") {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// `Description: [u16; 128]` 같은 UTF-16 슬라이스를 null 종료 기준 String으로.
/// null이 없으면(가득 채워진 경우) 전체 길이 사용. 양 끝 공백 trim.
fn decode_description(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end]).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reg_sz_intel() {
        let out = "\
\r
HKLM\\HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0\r
    ProcessorNameString    REG_SZ    Intel(R) Core(TM) i7-13700K\r
";
        assert_eq!(
            parse_reg_sz_value(out),
            Some("Intel(R) Core(TM) i7-13700K".to_string())
        );
    }

    #[test]
    fn parse_reg_sz_amd_with_extra_spaces() {
        let out = "    ProcessorNameString\tREG_SZ\tAMD Ryzen 7 7800X3D 8-Core Processor";
        assert_eq!(
            parse_reg_sz_value(out),
            Some("AMD Ryzen 7 7800X3D 8-Core Processor".to_string())
        );
    }

    #[test]
    fn parse_reg_sz_missing_none() {
        let out = "HKLM\\... no value here";
        assert_eq!(parse_reg_sz_value(out), None);
    }

    #[test]
    fn parse_reg_sz_empty_after_keyword_none() {
        let out = "    ProcessorNameString    REG_SZ    \r\n";
        assert_eq!(parse_reg_sz_value(out), None);
    }

    #[test]
    fn decode_description_null_terminated() {
        let mut buf = [0u16; 16];
        let s = "RTX 4070";
        for (i, c) in s.encode_utf16().enumerate() {
            buf[i] = c;
        }
        assert_eq!(decode_description(&buf), "RTX 4070");
    }

    #[test]
    fn decode_description_full_buffer_no_null() {
        let s = "0123456789012345";
        let buf: Vec<u16> = s.encode_utf16().collect();
        assert_eq!(buf.len(), 16);
        assert_eq!(decode_description(&buf), s);
    }

    #[test]
    fn decode_description_empty() {
        let buf: [u16; 0] = [];
        assert_eq!(decode_description(&buf), "");
    }
}
