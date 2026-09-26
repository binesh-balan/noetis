//! Part 4 spike: dump what Zoom / Teams expose through UI Automation, to see whether "who is
//! speaking" can be read. Run during a call, output to a file:
//!   cargo run --example uia_dump --no-default-features --features platform-default -- zoom teams > dump.txt
//! Lines marked `*` mention speaking/talking/muted.

#[cfg(not(windows))]
fn main() {
    eprintln!("uia_dump is Windows only");
}

#[cfg(windows)]
fn main() -> windows::core::Result<()> {
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
    use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomation};

    let filters: Vec<String> = {
        let args: Vec<String> = std::env::args().skip(1).map(|a| a.to_lowercase()).collect();
        if args.is_empty() { vec!["zoom".into(), "teams".into()] } else { args }
    };
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        let uia: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
        let walker = uia.RawViewWalker()?;
        let root = uia.GetRootElement()?;
        let mut win = walker.GetFirstChildElement(&root).ok();
        while let Some(w) = win {
            let name = w.CurrentName().map(|b| b.to_string()).unwrap_or_default();
            let class = w.CurrentClassName().map(|b| b.to_string()).unwrap_or_default();
            let hay = format!("{name} {class}").to_lowercase();
            if filters.iter().any(|f| hay.contains(f)) {
                println!("=== window: {name:?} class={class:?}");
                dump(&walker, &w, 1);
            }
            win = walker.GetNextSiblingElement(&w).ok();
        }
    }
    Ok(())
}

#[cfg(windows)]
unsafe fn dump(walker: &windows::Win32::UI::Accessibility::IUIAutomationTreeWalker, el: &windows::Win32::UI::Accessibility::IUIAutomationElement, depth: usize) {
    if depth > 60 {
        return;
    }
    let name = el.CurrentName().map(|b| b.to_string()).unwrap_or_default();
    let id = el.CurrentAutomationId().map(|b| b.to_string()).unwrap_or_default();
    let class = el.CurrentClassName().map(|b| b.to_string()).unwrap_or_default();
    let ctype = el.CurrentControlType().map(|t| t.0).unwrap_or(0);
    let offscreen = el.CurrentIsOffscreen().map(|b| b.as_bool()).unwrap_or(false);
    let lower = name.to_lowercase();
    let mark = if ["speaking", "talking", "muted", "unmuted"].iter().any(|k| lower.contains(k)) { '*' } else { ' ' };
    println!("{mark}{:indent$}[{ctype}] name={name:?} id={id:?} class={class:?}{}", "", if offscreen { " offscreen" } else { "" }, indent = depth * 2);
    let mut child = walker.GetFirstChildElement(el).ok();
    while let Some(c) = child {
        dump(walker, &c, depth + 1);
        child = walker.GetNextSiblingElement(&c).ok();
    }
}
