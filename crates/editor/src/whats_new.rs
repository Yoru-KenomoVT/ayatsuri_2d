// The two windows about versions: What's new, once on the first start of a
// new version (and from Help), with this version's part of the changelog
// built into the app and a way into the Discord; and the update window,
// once per launch when GitHub has a newer release, never over another
// dialog or in the middle of a drag.

use crate::i18n::{tr, trf};

pub(crate) const DISCORD: &str = "https://discord.gg/WyzDsXzdnh";

const CHANGELOG: &str = include_str!("../../../CHANGELOG.md");

/// This version's section of the changelog, without its heading. A test
/// build ("0.5.8-test.2") reads the section of the release it leads to.
pub(crate) fn section<'a>(md: &'a str, version: &str) -> Option<&'a str> {
    let want = version.split('-').next().unwrap_or(version).trim_start_matches('v');
    let mut at = None;
    let mut offset = 0;
    for line in md.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        let Some(head) = line.strip_prefix("## ") else { continue };
        if at.is_some() {
            return md.get(at?..start).map(str::trim);
        }
        if head.split_whitespace().next().map(|v| v.trim_start_matches('v')) == Some(want) {
            at = Some(offset);
        }
    }
    md.get(at?..).map(str::trim)
}

/// The Discord invite as a button, for What's new, About and Help.
pub(crate) fn discord_button(ui: &mut egui::Ui) {
    if ui
        .button(format!("{}  {}", egui_phosphor::regular::DISCORD_LOGO, tr("Join the Discord")))
        .on_hover_text(tr("Ideas, questions and help from other riggers"))
        .clicked()
    {
        ui.ctx().open_url(egui::OpenUrl::new_tab(DISCORD));
    }
}

/// Whether the update window has had its turn this launch.
#[derive(Default)]
pub(crate) struct UpdateWindow {
    pub(crate) open: bool,
    shown: bool,
}

/// The first start of this version: note it, and say whether What's new
/// should open (it has something to show).
pub(crate) fn first_start(prefs: &mut crate::prefs::Prefs) -> bool {
    let now = crate::update::current();
    if prefs.last_seen_version.as_deref() == Some(now.as_str()) {
        return false;
    }
    let show = section(CHANGELOG, &now).is_some();
    prefs.last_seen_version = Some(now);
    prefs.save();
    show
}

impl crate::panels::EditorUi {
    pub(crate) fn whats_new_window(&mut self, ctx: &egui::Context) {
        if !self.show_whats_new {
            return;
        }
        let version = crate::update::current();
        let mut open = true;
        crate::windows::Dialog::new(trf("What's new in {version}", &[("version", &version)]))
            .width(520.0)
            .open(&mut open)
            .show(ctx, |ui| {
                match section(CHANGELOG, &version) {
                    Some(md) => {
                        egui::ScrollArea::vertical()
                            .max_height(380.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| crate::update::markdown(ui, md));
                    }
                    None => {
                        ui.weak(tr("This version has no notes."));
                    }
                }
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(4.0);
                ui.label(tr("Ideas, questions, or something you rigged? Come talk on Discord."));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    discord_button(ui);
                    if ui.button(tr("Close")).clicked() {
                        self.show_whats_new = false;
                    }
                });
            });
        if !open {
            self.show_whats_new = false;
        }
    }

    /// Once per launch, when a newer release is known and the user has not
    /// skipped it: a window with its notes and Update now, Later (ask again
    /// next launch) and Skip this version. It waits for a quiet moment: no
    /// other dialog up and no button held.
    pub(crate) fn update_window(&mut self, ctx: &egui::Context) {
        let Some(latest) = self.update.available().cloned() else { return };
        if !self.update_win.shown {
            let skipped = self.prefs.update_dismissed.as_deref() == Some(latest.version.as_str());
            let busy = crate::windows::dialog_shown_lately(ctx)
                || self.show_about
                || ctx.input(|i| i.pointer.any_down());
            if skipped {
                self.update_win.shown = true;
            } else if !busy {
                self.update_win = UpdateWindow { open: true, shown: true };
            }
        }
        if !self.update_win.open {
            return;
        }
        let mut open = true;
        crate::windows::Dialog::new(tr("Update available")).width(500.0).open(&mut open).show(
            ctx,
            |ui| {
                ui.label(
                    egui::RichText::new(trf(
                        "Ayatsuri2D {version} is available (you have {current}).",
                        &[("version", &latest.version), ("current", &crate::update::current())],
                    ))
                    .strong(),
                );
                ui.add_space(6.0);
                if !latest.notes.trim().is_empty() {
                    egui::ScrollArea::vertical()
                        .max_height(300.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| crate::update::markdown(ui, &latest.notes));
                    ui.add_space(8.0);
                }
                ui.horizontal(|ui| {
                    crate::update::actions(ui, &mut self.update, &latest);
                    // while it downloads or waits for a restart the choice is made
                    if self.update.installing() || self.update.installed.is_some() {
                        return;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button(tr("Skip this version"))
                            .on_hover_text(tr("Don't ask about this release again"))
                            .clicked()
                        {
                            self.prefs.update_dismissed = Some(latest.version.clone());
                            self.prefs.save();
                            self.update_win.open = false;
                        }
                        if ui
                            .button(tr("Later"))
                            .on_hover_text(tr("Ask again the next time Ayatsuri2D starts"))
                            .clicked()
                        {
                            self.update_win.open = false;
                        }
                    });
                });
            },
        );
        if !open {
            self.update_win.open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MD: &str = "# Changelog\n\n## 0.5.8 (2026-10-24)\n\nNew things.\n\n### Added\n\n- a\n\n## 0.5.7 beta (2026-10-23)\n\nOld things.\n";

    #[test]
    fn a_version_finds_its_own_section() {
        assert_eq!(section(MD, "0.5.8"), Some("New things.\n\n### Added\n\n- a"));
        assert_eq!(section(MD, "0.5.8-test.3"), Some("New things.\n\n### Added\n\n- a"));
        assert_eq!(section(MD, "0.5.7"), Some("Old things."));
        assert_eq!(section(MD, "0.5.9"), None);
    }

    #[test]
    fn the_built_in_changelog_has_this_version() {
        assert!(section(CHANGELOG, env!("CARGO_PKG_VERSION")).is_some());
    }
}
