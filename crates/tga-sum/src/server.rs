//! Getting a model server to talk to: use the one running, or start one.
//!
//! **One models folder.** The account holder chooses it; it holds the
//! `.gguf` files and llama.cpp's `llama-server.exe`, each in it or one folder
//! down ([`server_in`], [`models_in`]), so unpacking a llama.cpp release or a
//! model into a subfolder of its own needs no setting.
//!
//! **Attach first.** A server already on the port with the right model — a
//! Gemma, or the file chosen — is used as it is and never stopped from here:
//! it is somebody else's process, and two copies of a 15 GB model do not fit
//! a 16 GB card. A server serving some other model is named and left alone.
//!
//! **Otherwise start one**, from the models folder or the paths in the
//! settings: `llama-server.exe` by absolute path (a bare name would let a
//! planted exe beside ours run with the user's rights), no console window,
//! its output to a log file. The [`Keeper`] holds the child, and the window
//! stops it after it has been idle a while and when it closes.
//!
//! **Loopback, whatever the options say.** `--host`, `--port` and the model
//! flags in the extra options are dropped and set here, so no setting can put
//! the server on the network.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::client::{Client, Props};
use crate::Error;

/// For Gemma 4 26B-A4B on a 16 GB card, as ReVision measured them: the whole
/// model on the card, 32K of context, one slot, the file read into memory
/// rather than mapped (mapped, Windows counted ~22 GB against the server and
/// the machine ran short).
pub const DEFAULT_OPTIONS: &str =
    "-c 32768 -ngl 99 -fa on --jinja --load-mode none -rea off -np 1 -b 2048 -ub 1024";

/// Loading 15 GB from a slow disk is minutes. ReVision allows ten.
const LOAD_DEADLINE: Duration = Duration::from_secs(10 * 60);

const POLL: Duration = Duration::from_millis(500);

/// What starting a server needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub exe: PathBuf,
    pub model: PathBuf,
    /// A vision model's projector, `--mmproj`, kept on the card.
    pub mmproj: Option<PathBuf>,
    /// `--chat-template-file`, for a model whose file carries none.
    pub template: Option<PathBuf>,
    /// Extra options; empty means [`DEFAULT_OPTIONS`].
    pub options: String,
    /// Where the server's own output goes.
    pub log: PathBuf,
}

impl Launch {
    /// The OCR on the same server and port as `gemma`, from the models
    /// folder, with [`crate::ocr::OPTIONS`].
    pub fn ocr(gemma: &Launch, models: &Path) -> Option<Launch> {
        let (model, mmproj, template) = crate::ocr::files_in(models)?;
        Some(Launch {
            exe: gemma.exe.clone(),
            model,
            mmproj: Some(mmproj),
            template,
            options: crate::ocr::OPTIONS.into(),
            log: gemma.log.clone(),
        })
    }
}

/// Which model a run needs on the port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Want {
    /// Gemma, to write the summary.
    Gemma,
    /// The OCR, to read the photos.
    Ocr,
}

impl Want {
    fn takes(self, model: &str) -> bool {
        match self {
            Want::Gemma => is_gemma(model),
            Want::Ocr => crate::ocr::is_ocr(model),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Want::Gemma => "Gemma",
            Want::Ocr => "the OCR",
        }
    }
}

/// The extra options as arguments, with every flag that would choose the
/// address, the port or the model taken out. Double quotes group.
pub fn arguments(options: &str) -> Vec<String> {
    let options = if options.trim().is_empty() {
        DEFAULT_OPTIONS
    } else {
        options
    };
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    let mut any = false;
    for c in options.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any {
                    words.push(std::mem::take(&mut word));
                    any = false;
                }
            }
            c => {
                word.push(c);
                any = true;
            }
        }
    }
    if any {
        words.push(word);
    }
    // Flags this module sets itself, each with the value after it.
    const OURS: [&str; 6] = ["--host", "--port", "-m", "--model", "--mmproj", "--api-key"];
    let mut out = Vec::new();
    let mut skip = false;
    for w in words {
        if skip {
            skip = false;
            continue;
        }
        let flag = w.split('=').next().unwrap_or_default();
        if OURS.contains(&flag) {
            skip = !w.contains('=');
            continue;
        }
        out.push(w);
    }
    out
}

/// `folder` and the folders directly in it, sorted: where a model or a
/// llama.cpp release unpacked into a folder of its own lands.
fn and_one_down(folder: &Path) -> Vec<PathBuf> {
    let mut sub: Vec<PathBuf> = std::fs::read_dir(folder)
        .map(|list| {
            list.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default();
    sub.sort();
    sub.insert(0, folder.to_path_buf());
    sub
}

fn lower_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// `llama-server.exe` in the models folder or one folder down.
pub fn server_in(folder: &Path) -> Option<PathBuf> {
    and_one_down(folder)
        .into_iter()
        .map(|d| d.join("llama-server.exe"))
        .find(|p| p.is_file())
}

/// The models a summary can load from the models folder or one folder down:
/// every `.gguf` but a vision projector, an embedding model, the OCR, and the
/// second and later pieces of a split model (llama.cpp is given the first).
/// Sorted by path, ignoring case.
pub fn models_in(folder: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = and_one_down(folder)
        .iter()
        .filter_map(|d| std::fs::read_dir(d).ok())
        .flat_map(|list| list.flatten().map(|e| e.path()))
        .filter(|p| {
            let name = lower_name(p);
            p.is_file()
                && name.ends_with(".gguf")
                && !name.contains("mmproj")
                && !name.contains("embedding")
                && !crate::ocr::is_ocr(&name)
                && !later_piece(&name)
        })
        .collect();
    found.sort_by_key(|p| p.to_string_lossy().to_lowercase());
    found
}

/// `…-00002-of-00003.gguf`: a piece of a split model other than the first.
fn later_piece(name: &str) -> bool {
    let Some(at) = name.rfind("-of-") else {
        return false;
    };
    let piece = &name[..at];
    let digits = &piece[piece.len().saturating_sub(5)..];
    digits.len() == 5 && digits.bytes().all(|b| b.is_ascii_digit()) && digits != "00001"
}

/// The model to load when none is chosen: the first Gemma, the prompts'
/// own, else the first there is.
pub fn default_model(models: &[PathBuf]) -> Option<PathBuf> {
    models
        .iter()
        .find(|p| is_gemma(&lower_name(p)))
        .or(models.first())
        .cloned()
}

/// Whether a model file name is a Gemma's.
pub fn is_gemma(model: &str) -> bool {
    model.to_lowercase().contains("gemma")
}

/// The server this process started, if it did, the model it loaded, and when
/// it was last used.
#[derive(Debug, Default)]
pub struct Keeper {
    child: Mutex<Option<Child>>,
    model: Mutex<Option<PathBuf>>,
    used: Mutex<Option<Instant>>,
}

impl Keeper {
    pub fn new() -> Self {
        Self::default()
    }

    /// A server with `want`'s model ready on `client`'s port: the one already
    /// there, or one started from `launch`. `say` narrates what is being
    /// waited for.
    ///
    /// **The card holds one model at a time**, so a server this process
    /// started with the other model is stopped first: the OCR makes way for
    /// Gemma and Gemma for the OCR. Somebody else's server is never stopped;
    /// if it has the wrong model, that is an error naming it.
    pub fn ensure(
        &self,
        client: &Client,
        launch: Option<&Launch>,
        want: Want,
        cancel: &AtomicBool,
        say: &mut dyn FnMut(String),
    ) -> Result<Props, Error> {
        let port = client.port();
        if let Some(launch) = launch {
            let ours = self.model.lock().unwrap().clone();
            if self.running() && ours.is_some_and(|m| m != launch.model) {
                say(format!(
                    "Unloading the model server to load {}",
                    want.name()
                ));
                self.stop();
            }
        }
        if !client.healthy() {
            if client.listening() {
                // Something is there and not ready: a server still loading.
                say(format!(
                    "Waiting for the model server on 127.0.0.1:{port} to finish loading"
                ));
                self.wait(client, cancel, None)?;
            } else if let Some(launch) = launch {
                self.start(client, launch, cancel, say)?;
            } else {
                return Err(Error::NoServer(port));
            }
        }
        let props = client.props()?;
        // A server started from the settings runs the model the settings
        // chose; only somebody else's is checked — and the file chosen is
        // right whatever its name.
        let chosen = launch.is_some_and(|l| {
            l.model
                .file_name()
                .is_some_and(|n| n.eq_ignore_ascii_case(props.model.as_str()))
        });
        if !self.running() && !chosen && !want.takes(&props.model) {
            let name = if props.model.is_empty() {
                "an unnamed model".to_string()
            } else {
                props.model.clone()
            };
            return Err(Error::Bad(format!(
                "The server on 127.0.0.1:{port} has {name} loaded, not {}. Stop it, or \
                 choose another port in Settings › Summaries.",
                want.name()
            )));
        }
        self.touch();
        Ok(props)
    }

    fn start(
        &self,
        client: &Client,
        launch: &Launch,
        cancel: &AtomicBool,
        say: &mut dyn FnMut(String),
    ) -> Result<(), Error> {
        for (what, path) in [("llama-server", &launch.exe), ("model", &launch.model)] {
            if !path.is_absolute() || !path.is_file() {
                return Err(Error::Bad(format!(
                    "The {what} file is not there: {}. Choose it in Settings › Summaries.",
                    path.display()
                )));
            }
        }
        if let Some(dir) = launch.log.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let log = File::options()
            .create(true)
            .append(true)
            .open(&launch.log)?;
        let mut command = Command::new(&launch.exe);
        command.arg("-m").arg(&launch.model).args([
            "--host",
            "127.0.0.1",
            "--port",
            &client.port().to_string(),
        ]);
        if let Some(mmproj) = &launch.mmproj {
            command.arg("--mmproj").arg(mmproj);
        }
        if let Some(template) = &launch.template {
            command.arg("--chat-template-file").arg(template);
        }
        command
            .args(arguments(&launch.options))
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let model = launch
            .model
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        say(format!("Starting {model} on 127.0.0.1:{}", client.port()));
        let child = command.spawn()?;
        *self.child.lock().unwrap() = Some(child);
        *self.model.lock().unwrap() = Some(launch.model.clone());
        let waited = self.wait(client, cancel, Some(&launch.log));
        if waited.is_err() {
            // Half-loaded is no use to anybody, and it holds the card.
            self.stop();
        }
        waited
    }

    /// Until the server says it is healthy, it dies, the deadline passes or
    /// the run is cancelled.
    fn wait(&self, client: &Client, cancel: &AtomicBool, log: Option<&Path>) -> Result<(), Error> {
        let deadline = Instant::now() + LOAD_DEADLINE;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
            if client.healthy() {
                return Ok(());
            }
            if let Some(code) = self.exited() {
                let see = log
                    .map(|l| format!("; see {}", l.display()))
                    .unwrap_or_default();
                return Err(Error::Bad(format!("llama-server stopped ({code}){see}")));
            }
            if Instant::now() > deadline {
                return Err(Error::Bad(
                    "the model server did not finish loading in 10 minutes".into(),
                ));
            }
            std::thread::sleep(POLL);
        }
    }

    /// The child's exit status, if there is a child and it has exited.
    fn exited(&self) -> Option<String> {
        let mut slot = self.child.lock().unwrap();
        let status = slot.as_mut()?.try_wait().ok()??;
        *slot = None;
        Some(status.to_string())
    }

    /// Whether a server this process started is still running.
    pub fn running(&self) -> bool {
        let mut slot = self.child.lock().unwrap();
        match slot.as_mut().map(|c| c.try_wait()) {
            Some(Ok(None)) => true,
            Some(_) => {
                *slot = None;
                false
            }
            None => false,
        }
    }

    /// The server was just used: the idle clock starts again.
    pub fn touch(&self) {
        *self.used.lock().unwrap() = Some(Instant::now());
    }

    /// How long the server this process started has gone unused; `None` when
    /// there is none to stop.
    pub fn idle(&self) -> Option<Duration> {
        if !self.running() {
            return None;
        }
        Some(
            self.used
                .lock()
                .unwrap()
                .map(|t| t.elapsed())
                .unwrap_or_default(),
        )
    }

    /// Stop the server this process started, if it did. Never anyone else's.
    pub fn stop(&self) {
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Keeper {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_options_are_the_gemma_defaults() {
        assert_eq!(
            arguments("  ")[..2],
            ["-c".to_string(), "32768".to_string()]
        );
    }

    #[test]
    fn no_option_can_choose_the_address_the_port_or_the_model() {
        let args =
            arguments("-c 8192 --host 0.0.0.0 --port=9000 -m other.gguf --jinja --api-key x");
        assert_eq!(args, vec!["-c", "8192", "--jinja"]);
    }

    #[test]
    fn quotes_group_a_value_with_spaces() {
        assert_eq!(
            arguments(r#"--chat-template-file "C:\my models\t.jinja" -fa on"#),
            vec!["--chat-template-file", r"C:\my models\t.jinja", "-fa", "on"]
        );
    }

    #[test]
    fn the_models_folder_yields_its_server_and_models_not_projectors_or_the_ocr() {
        let dir = std::env::temp_dir().join(format!("tga-sum-models-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for sub in ["llama-b6000-bin-win-cuda-x64", "paddleocr-vl", "qwen"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        assert_eq!(server_in(&dir), None);
        assert!(models_in(&dir).is_empty());
        assert_eq!(default_model(&[]), None);
        std::fs::write(
            dir.join("llama-b6000-bin-win-cuda-x64")
                .join("llama-server.exe"),
            b"",
        )
        .unwrap();
        for name in [
            "mmproj-gemma-4-26B-A4B-it.gguf",
            "Qwen3.8-27B-UD-IQ4_XS.gguf",
            "gemma-4-26B-A4B-it-qat-UD-Q4_K_XL.gguf",
            "nomic-embedding-v2.gguf",
            "paddleocr-vl/PaddleOCR-VL-1.5.gguf",
            "qwen/Big-Q8_0-00001-of-00002.gguf",
            "qwen/Big-Q8_0-00002-of-00002.gguf",
        ] {
            std::fs::write(dir.join(name), b"").unwrap();
        }
        assert!(server_in(&dir).unwrap().ends_with("llama-server.exe"));
        let names: Vec<String> = models_in(&dir).iter().map(|p| lower_name(p)).collect();
        assert_eq!(
            names,
            [
                "gemma-4-26b-a4b-it-qat-ud-q4_k_xl.gguf",
                "qwen3.8-27b-ud-iq4_xs.gguf",
                "big-q8_0-00001-of-00002.gguf"
            ]
        );
        // A Gemma first, whatever comes first by name.
        assert!(default_model(&models_in(&dir))
            .unwrap()
            .ends_with("gemma-4-26B-A4B-it-qat-UD-Q4_K_XL.gguf"));
        let qwen = vec![dir.join("Qwen3.8-27B-UD-IQ4_XS.gguf")];
        assert_eq!(default_model(&qwen), Some(qwen[0].clone()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_keeper_with_nothing_started_has_nothing_to_stop() {
        let k = Keeper::new();
        assert!(!k.running());
        assert_eq!(k.idle(), None);
        k.stop();
    }

    #[test]
    fn nothing_on_the_port_and_nothing_to_start_is_no_server() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let never = AtomicBool::new(false);
        let out = Keeper::new().ensure(&Client::new(port), None, Want::Gemma, &never, &mut |_| {});
        assert!(
            matches!(out, Err(Error::NoServer(p)) if p == port),
            "{out:?}"
        );
    }

    #[test]
    fn a_missing_exe_is_named_rather_than_spawned() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let launch = Launch {
            exe: std::env::temp_dir().join("no-such-llama-server.exe"),
            model: std::env::temp_dir().join("no-such.gguf"),
            mmproj: None,
            template: None,
            options: String::new(),
            log: std::env::temp_dir().join("tga-sum-server.log"),
        };
        let never = AtomicBool::new(false);
        let out = Keeper::new().ensure(
            &Client::new(port),
            Some(&launch),
            Want::Gemma,
            &never,
            &mut |_| {},
        );
        match out {
            Err(Error::Bad(m)) => assert!(m.contains("llama-server file is not there"), "{m}"),
            other => panic!("{other:?}"),
        }
    }
}
