use std::sync::mpsc::{self, Receiver, Sender};

use eframe::egui;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Profile(u8),
    MuteMics,
    MuteAll,
}

pub struct Hotkeys {
    rx: Receiver<Action>,
}

impl Hotkeys {
    pub fn start(ctx: egui::Context) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || listen(tx, ctx));
        Self { rx }
    }

    pub fn poll(&self) -> Vec<Action> {
        self.rx.try_iter().collect()
    }
}

#[cfg(windows)]
fn listen(tx: Sender<Action>, ctx: egui::Context) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        RegisterHotKey, HOT_KEY_MODIFIERS, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG, WM_HOTKEY};

    const VK_F9: u32 = 0x78;
    const VK_F10: u32 = 0x79;

    let mut bindings: Vec<(i32, Action, String, HOT_KEY_MODIFIERS, u32)> = (1
        ..=crate::MAX_SHORTCUT_SLOT)
        .map(|slot| {
            (
                i32::from(slot),
                Action::Profile(slot),
                crate::shortcut_label(slot),
                MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT,
                0x30 + u32::from(slot % 10),
            )
        })
        .collect();
    bindings.push((
        i32::from(crate::MAX_SHORTCUT_SLOT) + 1,
        Action::MuteMics,
        "F9".to_string(),
        MOD_NOREPEAT,
        VK_F9,
    ));
    bindings.push((
        i32::from(crate::MAX_SHORTCUT_SLOT) + 2,
        Action::MuteAll,
        "F10".to_string(),
        MOD_NOREPEAT,
        VK_F10,
    ));

    for (id, _, label, modifiers, key) in &bindings {
        if let Err(err) = unsafe { RegisterHotKey(None, *id, *modifiers, *key) } {
            crate::logging::log("hotkey", &format!("{label} indisponible: {err}"));
        }
    }

    loop {
        let mut message = MSG::default();
        let status = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if status.0 <= 0 {
            break;
        }
        if message.message != WM_HOTKEY {
            continue;
        }
        let id = message.wParam.0 as i32;
        let Some((_, action, _, _, _)) =
            bindings.iter().find(|(binding, _, _, _, _)| *binding == id)
        else {
            continue;
        };
        if tx.send(*action).is_err() {
            break;
        }
        ctx.request_repaint();
    }
}

#[cfg(not(windows))]
fn listen(_tx: Sender<Action>, _ctx: egui::Context) {}
