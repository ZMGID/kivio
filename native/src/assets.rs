//! Keep Kit's component icons and embed only Kivio's extra navigation icons.
use gpui_kit::*;
use std::borrow::Cow;

gpui_kit::assets::icon_assets!(
    NavigationIcons,
    [
        Archive,
        Briefcase,
        Keyboard,
        Layers,
        ListTodo,
        Lock,
        MessageSquare,
        Puzzle,
        SquarePen
    ]
);
pub struct Assets;
impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == "kivio/momo.svg" {
            return Ok(Some(Cow::Borrowed(include_bytes!("../assets/momo.svg"))));
        }
        if path == "kivio/logo-mark.png" {
            return Ok(Some(Cow::Borrowed(include_bytes!(
                "../../public/logo-mark.png"
            ))));
        }
        if let Some(icon) = NavigationIcons.load(path)? {
            return Ok(Some(icon));
        }
        gpui_kit::assets::Assets.load(path)
    }
    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(NavigationIcons.list(path)?);
        if "kivio/logo-mark.png".starts_with(path) {
            paths.push("kivio/logo-mark.png".into());
        }
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}
