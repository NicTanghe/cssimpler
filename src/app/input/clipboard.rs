/// Keep the clipboard alive for the app's lifetime (X11/Wayland own the data).
/// Initialization is lazy so headless apps and tests need no desktop session.
#[derive(Default)]
pub(super) struct TextClipboard {
    system: Option<arboard::Clipboard>,
    #[cfg(test)]
    pub memory: Option<Result<String, ()>>,
}

impl TextClipboard {
    pub fn get_text(&mut self) -> Option<String> {
        #[cfg(test)]
        if let Some(value) = &self.memory {
            return value.clone().ok();
        }
        self.system()?.get_text().ok()
    }

    pub fn set_text(&mut self, text: String) -> bool {
        #[cfg(test)]
        if let Some(value) = &mut self.memory {
            if value.is_err() {
                return false;
            }
            *value = Ok(text);
            return true;
        }
        self.system()
            .is_some_and(|clipboard| clipboard.set_text(text).is_ok())
    }

    fn system(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.system.is_none() {
            self.system = arboard::Clipboard::new().ok();
        }
        self.system.as_mut()
    }
}
