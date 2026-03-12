use {
    super::*,
    gpui::{AssetSource, SharedString},
    std::borrow::Cow,
};

pub(crate) fn find_assets_root_dir() -> Option<PathBuf> {
    if let Ok(exe) = env::current_exe() {
        let exe_dir = exe.parent()?;

        let macos_bundle = exe_dir.parent().map(|path| path.join("Resources"));
        if let Some(dir) = macos_bundle
            && dir.is_dir()
        {
            return Some(dir);
        }

        let share_dir = exe_dir
            .parent()
            .map(|path| path.join("share").join("arbor"));
        if let Some(dir) = share_dir
            && dir.is_dir()
        {
            return Some(dir);
        }
    }

    let dev_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    if dev_dir.is_dir() {
        return Some(dev_dir);
    }

    None
}

pub(crate) fn find_asset_dir(relative_subdir: &str) -> Option<PathBuf> {
    let dir = find_assets_root_dir()?.join(relative_subdir);
    dir.is_dir().then_some(dir)
}

pub(crate) fn find_top_bar_icons_dir() -> Option<PathBuf> {
    static TOP_BAR_ICONS_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();

    TOP_BAR_ICONS_DIR
        .get_or_init(|| find_asset_dir("icons/top-bar"))
        .clone()
}

pub(crate) fn resolve_embedded_terminal_engine(
    configured: Option<&str>,
    notices: &mut Vec<String>,
) -> arbor_terminal_emulator::TerminalEngineKind {
    let requested = env::var("ARBOR_TERMINAL_ENGINE").ok();
    match arbor_terminal_emulator::parse_terminal_engine_kind(requested.as_deref().or(configured)) {
        Ok(engine) => {
            arbor_terminal_emulator::set_default_terminal_engine(engine);
            engine
        },
        Err(error) => {
            notices.push(error);
            let engine = arbor_terminal_emulator::TerminalEngineKind::default();
            arbor_terminal_emulator::set_default_terminal_engine(engine);
            engine
        },
    }
}

pub(crate) struct ArborAssets {
    pub(crate) base: PathBuf,
}

impl AssetSource for ArborAssets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        fs::read(self.base.join(path))
            .map(|data| Some(Cow::Owned(data)))
            .map_err(Into::into)
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        fs::read_dir(self.base.join(path))
            .map(|entries| {
                entries
                    .filter_map(|entry| {
                        entry
                            .ok()
                            .and_then(|entry| entry.file_name().into_string().ok())
                            .map(SharedString::from)
                    })
                    .collect()
            })
            .map_err(Into::into)
    }
}
