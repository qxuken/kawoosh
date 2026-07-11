use std::borrow::Cow;

use anyhow::{Context, Result};
use gpui::{AssetSource, SharedString};
use log::{debug, trace};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "../assets/"]
#[exclude = "*.DS_Store"]
pub struct Resources;

impl AssetSource for Resources {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Self::get(path)
            .map(|r| Some(r.data))
            .with_context(|| format!("loading asset at path {path:?}"))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(Self::iter()
            .filter(|f| f.starts_with(path))
            .map(|f| f.into())
            .collect())
    }
}

impl Resources {
    pub fn load_fonts(cx: &mut gpui::App) -> Result<()> {
        trace!("load_fonts::loading");
        let fonts = Self::iter()
            .filter(|f| f.starts_with("fonts") && f.ends_with(".ttf"))
            .map(|f| Self::get(&f).expect("Load from static storage"))
            .map(|f| f.data)
            .collect();
        trace!("load_fonts::retrieved");
        let res = cx.text_system().add_fonts(fonts);
        debug!("load_fonts::loaded");
        res
    }
}
