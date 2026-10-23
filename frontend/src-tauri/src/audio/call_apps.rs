//! Shared knowledge about the call apps Meetily can record on its own (Zoom, Teams).
//!
//! The recording side taps only the audio processes of one call app; the detector side asks which
//! call apps are running. Both use the types and lookups here.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CallAppId {
    Zoom,
    Teams,
}

impl CallAppId {
    pub const ALL: [CallAppId; 2] = [CallAppId::Zoom, CallAppId::Teams];

    pub fn label(self) -> &'static str {
        match self {
            CallAppId::Zoom => "Zoom",
            CallAppId::Teams => "Teams",
        }
    }

    pub fn bundle_ids(self) -> &'static [&'static str] {
        match self {
            CallAppId::Zoom => &["us.zoom.xos"],
            CallAppId::Teams => &["com.microsoft.teams2", "com.microsoft.teams"],
        }
    }

    /// "zoom" or "teams", case-insensitive, surrounding whitespace ignored.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "zoom" => Some(CallAppId::Zoom),
            "teams" => Some(CallAppId::Teams),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            CallAppId::Zoom => "zoom",
            CallAppId::Teams => "teams",
        }
    }
}

/// One Core Audio HAL process object.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioProcess {
    pub object_id: u32,
    pub pid: i32,
    pub bundle_id: Option<String>,
    pub exe_path: Option<PathBuf>,
    pub running_input: bool,
    pub running_output: bool,
}

/// A running application that has a bundle id.
#[derive(Debug, Clone, PartialEq)]
pub struct RunningApp {
    pub bundle_id: String,
    pub bundle_path: PathBuf,
    pub pid: i32,
}

/// Pure. A process belongs to `app` only when its executable lies inside, by whole path
/// components, the bundle folder of a running app whose bundle id is one of `app`'s ids.
/// A process's own bundle-id claim never counts by itself (helpers report bogus ids).
pub fn owned_by(app: CallAppId, procs: &[AudioProcess], apps: &[RunningApp]) -> Vec<AudioProcess> {
    let bundles: Vec<&Path> = apps
        .iter()
        .filter(|a| app.bundle_ids().contains(&a.bundle_id.as_str()))
        .map(|a| a.bundle_path.as_path())
        .collect();
    procs
        .iter()
        .filter(|p| {
            p.exe_path
                .as_deref()
                .is_some_and(|exe| bundles.iter().any(|b| exe.starts_with(b)))
        })
        .cloned()
        .collect()
}

/// Snapshot of Core Audio's HAL process objects (per-process PID, bundle id, IsRunningInput,
/// IsRunningOutput; exe path from the PID).
#[cfg(target_os = "macos")]
pub fn audio_processes() -> anyhow::Result<Vec<AudioProcess>> {
    use cidre::core_audio as ca;

    let list = ca::Process::list()
        .map_err(|e| anyhow::anyhow!("Failed to list Core Audio processes: {:?}", e))?;
    Ok(list
        .iter()
        .map(|p| {
            let pid = p.pid().unwrap_or(-1);
            AudioProcess {
                object_id: p.0 .0,
                pid,
                bundle_id: p.bundle_id().ok().map(|b| b.to_string()),
                exe_path: exe_path_of_pid(pid),
                running_input: p.is_running_input().unwrap_or(false),
                running_output: p.is_running_output().unwrap_or(false),
            }
        })
        .collect())
}

#[cfg(not(target_os = "macos"))]
pub fn audio_processes() -> anyhow::Result<Vec<AudioProcess>> {
    Ok(vec![])
}

/// Running apps that have a bundle id, with their bundle folder (NSWorkspace running applications).
#[cfg(target_os = "macos")]
pub fn running_apps() -> Vec<RunningApp> {
    use cidre::{ns, objc};

    objc::ar_pool(|| {
        let ws = ns::Workspace::shared();
        let apps = ws.running_apps();
        let mut out = Vec::new();
        for app in apps.iter() {
            let (Some(bundle_id), Some(url)) = (app.bundle_id(), app.bundle_url()) else {
                continue;
            };
            let Some(path) = url.path() else { continue };
            out.push(RunningApp {
                bundle_id: bundle_id.to_string(),
                bundle_path: PathBuf::from(path.to_string()),
                pid: app.pid(),
            });
        }
        out
    })
}

#[cfg(not(target_os = "macos"))]
pub fn running_apps() -> Vec<RunningApp> {
    vec![]
}

#[cfg(target_os = "macos")]
fn exe_path_of_pid(pid: i32) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;

    extern "C" {
        // libproc, part of libSystem: no extra crate needed.
        fn proc_pidpath(pid: i32, buffer: *mut u8, buffersize: u32) -> i32;
    }
    if pid <= 0 {
        return None;
    }
    let mut buf = vec![0u8; 4096];
    // SAFETY: buf is a live, writable buffer of exactly buf.len() bytes, and proc_pidpath writes at most that many.
    let n = unsafe { proc_pidpath(pid, buf.as_mut_ptr(), buf.len() as u32) };
    if n <= 0 {
        return None;
    }
    buf.truncate(n as usize);
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(&buf)))
}

/// Core Audio process object ids that belong to `app` right now. Every owned process counts, not
/// only those currently outputting, since a call app starts output in a helper later.
pub fn owned_object_ids(app: CallAppId) -> anyhow::Result<Vec<u32>> {
    let procs = audio_processes()?;
    let mut ids: Vec<u32> = owned_by(app, &procs, &running_apps())
        .into_iter()
        .map(|p| p.object_id)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct CallAppInfo {
    pub id: CallAppId,
    pub label: &'static str,
    pub running: bool,
}

pub fn call_app_infos(apps: &[RunningApp]) -> Vec<CallAppInfo> {
    CallAppId::ALL
        .iter()
        .map(|&id| CallAppInfo {
            id,
            label: id.label(),
            running: apps.iter().any(|a| id.bundle_ids().contains(&a.bundle_id.as_str())),
        })
        .collect()
}

/// The call apps Meetily can record on its own, and whether each is running.
#[tauri::command]
pub async fn list_call_apps() -> Result<Vec<CallAppInfo>, String> {
    Ok(call_app_infos(&running_apps()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(exe: Option<&str>, bundle: Option<&str>) -> AudioProcess {
        AudioProcess {
            object_id: 1,
            pid: 10,
            bundle_id: bundle.map(String::from),
            exe_path: exe.map(PathBuf::from),
            running_input: false,
            running_output: true,
        }
    }

    fn app(bundle_id: &str, path: &str) -> RunningApp {
        RunningApp { bundle_id: bundle_id.into(), bundle_path: PathBuf::from(path), pid: 99 }
    }

    #[test]
    fn zoom_main_process_is_owned_by_zoom_only() {
        let p = [proc(Some("/Applications/zoom.us.app/Contents/MacOS/zoom.us"), None)];
        let apps = [app("us.zoom.xos", "/Applications/zoom.us.app")];
        assert_eq!(owned_by(CallAppId::Zoom, &p, &apps), p.to_vec());
        assert!(owned_by(CallAppId::Teams, &p, &apps).is_empty());
    }

    #[test]
    fn teams_nested_helper_is_owned_by_teams() {
        let p = [proc(
            Some("/Applications/Microsoft Teams.app/Contents/Helpers/Microsoft Teams WebView.app/Contents/MacOS/Microsoft Teams WebView"),
            None,
        )];
        let apps = [app("com.microsoft.teams2", "/Applications/Microsoft Teams.app")];
        assert_eq!(owned_by(CallAppId::Teams, &p, &apps), p.to_vec());
    }

    #[test]
    fn bundle_id_claim_alone_does_not_count() {
        let p = [proc(Some("/usr/libexec/some-helper"), Some("us.zoom.xos"))];
        let apps = [app("us.zoom.xos", "/Applications/zoom.us.app")];
        assert!(owned_by(CallAppId::Zoom, &p, &apps).is_empty());
    }

    #[test]
    fn prefix_must_be_whole_path_components() {
        let p = [proc(Some("/Applications/zoom.us.app.old/Contents/MacOS/zoom.us"), None)];
        let apps = [app("us.zoom.xos", "/Applications/zoom.us.app")];
        assert!(owned_by(CallAppId::Zoom, &p, &apps).is_empty());
    }

    #[test]
    fn missing_exe_is_not_owned() {
        let p = [proc(None, Some("us.zoom.xos"))];
        let apps = [app("us.zoom.xos", "/Applications/zoom.us.app")];
        assert!(owned_by(CallAppId::Zoom, &p, &apps).is_empty());
    }

    #[test]
    fn no_running_app_with_the_bundle_id_means_not_owned() {
        let p = [proc(Some("/Applications/zoom.us.app/Contents/MacOS/zoom.us"), None)];
        let apps = [app("com.example.other", "/Applications/Other.app")];
        assert!(owned_by(CallAppId::Zoom, &p, &apps).is_empty());
        assert!(owned_by(CallAppId::Zoom, &p, &[]).is_empty());
    }

    #[test]
    fn classic_teams_is_owned_by_teams() {
        let p = [proc(Some("/Applications/Microsoft Teams classic.app/Contents/MacOS/Teams"), None)];
        let apps = [app("com.microsoft.teams", "/Applications/Microsoft Teams classic.app")];
        assert_eq!(owned_by(CallAppId::Teams, &p, &apps), p.to_vec());
    }

    #[test]
    fn parse_is_case_insensitive_and_trimmed() {
        assert_eq!(CallAppId::parse(" Zoom "), Some(CallAppId::Zoom));
        assert_eq!(CallAppId::parse("TEAMS"), Some(CallAppId::Teams));
        assert_eq!(CallAppId::parse("webex"), None);
        for id in CallAppId::ALL {
            assert_eq!(CallAppId::parse(id.as_str()), Some(id));
        }
    }

    #[test]
    fn call_app_infos_marks_running_apps() {
        let apps = [app("com.microsoft.teams", "/Applications/Microsoft Teams classic.app")];
        let infos = call_app_infos(&apps);
        assert_eq!(infos.len(), 2);
        assert!(!infos[0].running && infos[0].id == CallAppId::Zoom);
        assert!(infos[1].running && infos[1].id == CallAppId::Teams);
        let json = serde_json::to_string(&infos[0]).unwrap();
        assert_eq!(json, r#"{"id":"zoom","label":"Zoom","running":false}"#);
    }
}
