//! Wireless: a Ferrite radio for internet streams.
//!
//! A dial across an FM-style band with stations on it. Turn it and you pass
//! through static; land near a station and it fades in. A VU meter shows the
//! level, the station and the track scroll by on a marquee, and closing the
//! window switches the set off like an old CRT.
//!
//!     cargo run
//!     cargo run -- --off        # start switched off

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod dial;
mod settings;

use std::sync::Arc;
use std::time::Duration;

use ferrite_design::prelude::*;
use gpui::{App, AppContext as _, ClickEvent, Context, Entity, IntoElement, KeyBinding, Render, Subscription, Task, Window, div, px, size};

use audio::{Radio, Status};
use dial::{Station, Tuning};
use settings::Settings;

/// How often the meters update while the set is on.
const METER_TICK: Duration = Duration::from_millis(40);
/// How long the dial must rest on a station before it's tuned in.
const SETTLE: Duration = Duration::from_millis(350);
/// A seek sweeps the dial in this many steps, this far apart.
const SWEEP_STEPS: u32 = 24;
const SWEEP_STEP: Duration = Duration::from_millis(28);
/// The VU needle's fall per tick (rise is instant).
const FALL: f32 = 0.82;
/// The dial's widest face, in characters; it's as wide as the window
/// allows from `dial::MIN_SCALE_COLS` up.
const MAX_SCALE_COLS: usize = 240;

const APPEARANCES: [(&str, &str); 3] = [("dark", "Dark"), ("light", "Light"), ("system", "System")];
const FPS: [u32; 4] = [25, 60, 120, 240];

struct Wireless {
    settings: Settings,
    settings_open: bool,
    band: Vec<Station>,
    /// The dial, MHz (unsnapped while sweeping).
    freq: f32,
    on: bool,
    radio: Arc<Radio>,
    /// The station whose stream is open.
    playing: Option<usize>,
    /// How many times a station has been tuned in (replays the name effect).
    tuned: u64,
    vu: [f32; 2],
    meter_task: Option<Task<()>>,
    settle_task: Option<Task<()>>,
    sweep_task: Option<Task<()>>,
    palette: Entity<CommandPalette>,
    toaster: Entity<Toaster>,
    _appearance: Subscription,
}

impl Wireless {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = Settings::load();
        let radio = Radio::start(settings.volume);
        let mut set = Self {
            band: settings.band(),
            freq: settings.freq,
            settings,
            settings_open: std::env::args().any(|a| a == "--settings"),
            on: false,
            radio,
            playing: None,
            tuned: 0,
            vu: [0.; 2],
            meter_task: None,
            settle_task: None,
            sweep_task: None,
            palette: cx.new(|cx| CommandPalette::new(window, cx)),
            toaster: cx.new(|_| Toaster::new()),
            _appearance: theme::follow_system(window),
        };
        set.apply_look(window, cx);
        // Lodestone hands over its shared look as FERRITE_* variables; when
        // launched that way, those win over the saved settings.
        theme::apply_env(cx);
        set.set_commands(cx);
        set.power(!std::env::args().any(|a| a == "--off"), cx);
        set
    }

    // ── Settings ─────────────────────────────────────────────────────────

    fn apply_look(&self, window: &mut Window, cx: &mut App) {
        let s = &self.settings;
        if let Some(scheme) = schemes::by_key(&s.scheme) {
            theme::set_scheme(scheme, cx);
        }
        let appearance = match s.appearance.as_str() {
            "light" => Appearance::Light,
            "system" => Appearance::System,
            _ => Appearance::Dark,
        };
        theme::set_appearance(appearance, window, cx);
        motion::set_fps(s.fps);
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if let Err(err) = self.settings.save() {
            self.toaster.update(cx, |t, cx| t.push(toast("Couldn't save settings").danger().message(err), cx));
        }
        cx.notify();
    }

    fn change(&mut self, f: impl FnOnce(&mut Settings), window: &mut Window, cx: &mut Context<Self>) {
        f(&mut self.settings);
        self.apply_look(window, cx);
        self.save(cx);
    }

    fn set_commands(&self, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        let run = |f: fn(&mut Self, &mut Window, &mut Context<Self>)| {
            let weak = weak.clone();
            move |window: &mut Window, cx: &mut App| {
                let _ = weak.update(cx, |this, cx| f(this, window, cx));
            }
        };
        let mut commands = vec![
            command("Power on / off").group("Radio").icon(Icon::Play).shortcut("Space").on_run(run(|this, _, cx| this.power(!this.on, cx))),
            command("Seek up").group("Radio").icon(Icon::ChevronRight).shortcut("Right").on_run(run(|this, _, cx| this.seek(true, cx))),
            command("Seek down").group("Radio").icon(Icon::ChevronLeft).shortcut("Left").on_run(run(|this, _, cx| this.seek(false, cx))),
            command("Volume up").group("Radio").icon(Icon::Plus).shortcut("Up").on_run(run(|this, _, cx| this.nudge_volume(0.05, cx))),
            command("Volume down").group("Radio").icon(Icon::Minus).shortcut("Down").on_run(run(|this, _, cx| this.nudge_volume(-0.05, cx))),
            command("Settings").group("Radio").icon(Icon::Sliders).shortcut("Ctrl+,").on_run(run(|this, _, cx| {
                this.settings_open = true;
                cx.notify();
            })),
            command("Dark theme").group("Theme").on_run(run(|this, window, cx| this.change(|s| s.appearance = "dark".into(), window, cx))),
            command("Light theme").group("Theme").on_run(run(|this, window, cx| this.change(|s| s.appearance = "light".into(), window, cx))),
        ];
        for (i, st) in self.band.iter().enumerate() {
            let weak = weak.clone();
            commands.push(command(format!("Tune: {:.1} {}", st.freq, st.name)).group("Stations").icon(Icon::Dot).on_run(move |_, cx| {
                let _ = weak.update(cx, |this, cx| this.sweep_to(this.band[i].freq, cx));
            }));
        }
        for scheme in SCHEMES {
            let weak = weak.clone();
            commands.push(command(format!("Scheme: {}", scheme.name)).group("Theme").on_run(move |window, cx| {
                let _ = weak.update(cx, |this, cx| this.change(|s| s.scheme = scheme.key.into(), window, cx));
            }));
        }
        self.palette.update(cx, |p, cx| p.set_commands(commands, cx));
    }

    // ── The set ──────────────────────────────────────────────────────────

    fn tuning(&self) -> Tuning {
        dial::tune(self.freq, &self.band)
    }

    fn power(&mut self, on: bool, cx: &mut Context<Self>) {
        self.on = on;
        self.radio.set_on(on);
        if on {
            self.settle(cx);
            self.meter_task = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(METER_TICK).await;
                    if this.update(cx, |this, cx| this.meter(cx)).is_err() {
                        break;
                    }
                }
            }));
        } else {
            self.radio.stop();
            self.playing = None;
            self.meter_task = None;
            self.settle_task = None;
            self.vu = [0.; 2];
        }
        cx.notify();
    }

    /// Read the levels, let the needles fall, and keep the static in step
    /// with what's actually coming through.
    fn meter(&mut self, cx: &mut Context<Self>) {
        let peaks = self.radio.take_peaks();
        for (v, p) in self.vu.iter_mut().zip(peaks) {
            *v = p.max(*v * FALL);
        }
        let t = self.tuning();
        let through = self.playing.is_some() && self.playing == t.station && self.radio.status() == Status::Playing;
        self.radio.set_hiss(if through { t.hiss() } else { 1. });
        cx.notify();
    }

    /// Move the dial. The station follows once it rests there.
    fn set_freq(&mut self, freq: f32, cx: &mut Context<Self>) {
        self.freq = freq.clamp(dial::LOW, dial::HIGH);
        self.settle(cx);
        cx.notify();
    }

    fn settle(&mut self, cx: &mut Context<Self>) {
        if !self.on {
            return;
        }
        self.settle_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SETTLE).await;
            let _ = this.update(cx, |this, cx| this.lock(cx));
        }));
    }

    /// Open the stream for the station the dial is on, if it isn't already.
    fn lock(&mut self, cx: &mut Context<Self>) {
        let want = self.tuning().locked();
        if want != self.playing {
            match want {
                Some(i) => {
                    self.radio.play(self.band[i].url.clone());
                    self.tuned += 1;
                }
                None => self.radio.stop(),
            }
            self.playing = want;
        }
        let freq = dial::snap(self.freq);
        if self.settings.freq != freq {
            self.settings.freq = freq;
            self.save(cx);
        }
        cx.notify();
    }

    /// Sweep the dial to `target`, through whatever static lies between.
    fn sweep_to(&mut self, target: f32, cx: &mut Context<Self>) {
        let from = self.freq;
        self.sweep_task = Some(cx.spawn(async move |this, cx| {
            for step in 1..=SWEEP_STEPS {
                cx.background_executor().timer(SWEEP_STEP).await;
                // Ease out: fast off the old station, slow onto the new one.
                let t = step as f32 / SWEEP_STEPS as f32;
                let eased = 1. - (1. - t).powi(3);
                let f = if step == SWEEP_STEPS { target } else { from + (target - from) * eased };
                if this.update(cx, |this, cx| this.set_freq(f, cx)).is_err() {
                    return;
                }
            }
        }));
    }

    fn seek(&mut self, up: bool, cx: &mut Context<Self>) {
        if let Some(i) = dial::seek(self.freq, &self.band, up) {
            self.sweep_to(self.band[i].freq, cx);
        }
    }

    fn set_volume(&mut self, v: f32, cx: &mut Context<Self>) {
        self.settings.volume = v.clamp(0., 1.);
        self.radio.set_volume(self.settings.volume);
        self.save(cx);
    }

    fn nudge_volume(&mut self, by: f32, cx: &mut Context<Self>) {
        self.set_volume(((self.settings.volume + by) * 20.).round() / 20., cx);
    }

    // ── Views ────────────────────────────────────────────────────────────

    /// What the marquee says.
    fn ticker(&self) -> String {
        if !self.on {
            return "WIRELESS · OFF · PRESS SPACE TO SWITCH ON".into();
        }
        let t = self.tuning();
        match t.station {
            None => format!("{:.1} FM · NOTHING BUT STATIC · SEEK WITH THE ARROW KEYS", dial::snap(self.freq)),
            Some(i) => {
                let st = &self.band[i];
                let state = match self.radio.status() {
                    _ if t.locked().is_none() => "TOO FAINT TO LOCK ON".to_string(),
                    Status::Connecting | Status::Idle => "TUNING IN".to_string(),
                    Status::Playing => self.radio.title().map(|t| format!("NOW PLAYING {t}")).unwrap_or_else(|| "ON AIR".into()),
                    Status::Failed(why) => format!("NO SIGNAL: {why}"),
                };
                format!("{:.1} FM · {} · {} · {}", st.freq, st.name, st.genre, state).to_uppercase()
            }
        }
    }

    fn settings_drawer(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let p = palette(cx);
        let s = &self.settings;
        let scheme_index = SCHEMES.iter().position(|sc| sc.key == s.scheme);
        let close = {
            let weak = cx.weak_entity();
            move |_: &mut Window, cx: &mut App| {
                let _ = weak.update(cx, |this, cx| {
                    this.settings_open = false;
                    cx.notify();
                });
            }
        };
        let path = Settings::path().map(|p| p.display().to_string()).unwrap_or_else(|| "wireless.conf".into());
        let mut stations = property_list();
        for st in &self.band {
            stations = stations.row(format!("{:.1}", st.freq), st.name.clone());
        }
        drawer("settings")
            .open(self.settings_open)
            .title("Settings")
            .width(px(420.))
            .on_close(close)
            .child(rule(Some("look"), window, cx))
            .child(
                field("scheme", "Scheme").child(
                    select("scheme-select")
                        .options(SCHEMES.iter().map(|sc| sc.name))
                        .selected(scheme_index)
                        .width(px(220.))
                        .on_change(cx.listener(|this, i: &usize, window, cx| this.change(|s| s.scheme = SCHEMES[*i].key.into(), window, cx))),
                ),
            )
            .child(
                field("appearance", "Appearance").child(
                    APPEARANCES.iter().fold(segmented("appearance-seg"), |seg, (_, label)| seg.option(*label))
                        .selected(APPEARANCES.iter().position(|(k, _)| *k == s.appearance).unwrap_or(0))
                        .on_select(cx.listener(|this, i: &usize, window, cx| this.change(|s| s.appearance = APPEARANCES[*i].0.into(), window, cx))),
                ),
            )
            .child(
                field("fps", "Refresh rate").hint("25 is the classic stepped look").child(
                    FPS.iter().fold(segmented("fps-seg"), |seg, f| seg.option(f.to_string()))
                        .selected(FPS.iter().position(|f| *f == s.fps).unwrap_or(FPS.len() - 1))
                        .on_select(cx.listener(|this, i: &usize, window, cx| this.change(|s| s.fps = FPS[*i], window, cx))),
                ),
            )
            .child(rule(Some("stations"), window, cx))
            .child(div().body(text::SM).text_color(hsla(p.fg_dim)).child(format!(
                "{} on the dial{}. Add your own as `station = 94.1 | Name | genre | url` lines in {path}; any you add replace the presets.",
                self.band.len(),
                if s.stations.is_empty() { ", the SomaFM presets" } else { "" }
            )))
            .child(stations)
    }
}

impl Render for Wireless {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The window is closing: the set goes quiet as the picture collapses.
        if chrome::powering_off(window, cx) {
            self.radio.set_on(false);
        }
        let p = palette(cx);
        let t = self.tuning();
        let on = self.on;
        let status = self.radio.status();
        let station = t.station.map(|i| &self.band[i]);

        // gpui has no zoom: the dial, meters and ticker take the window's
        // width, and the frequency goes up a size when there's room.
        let viewport = window.viewport_size();
        let cell = f32::from(display_size(Scale::X1, window)) / 2.;
        // Less the slider's readout ("107.9 MHz") to the right of the track.
        let room_cols = ((f32::from(viewport.width) - 2. * f32::from(space::ROW) - 48.) / cell) as usize - 12;
        let scale_cols = room_cols.clamp(dial::MIN_SCALE_COLS, MAX_SCALE_COLS);
        let roomy = f32::from(viewport.width) >= 1400. && f32::from(viewport.height) >= 800.;
        let ink = if on { p.fg } else { p.fg_faint };
        let name = match station {
            Some(st) if on => st.name.to_uppercase(),
            _ if on => "STATIC".into(),
            _ => "OFF".into(),
        };
        let genre = station.filter(|_| on).map(|s| s.genre.clone()).unwrap_or_default();
        let readout = div()
            .flex()
            .flex_row()
            .items_end()
            .justify_between()
            .gap_6()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_end()
                    .gap_3()
                    .child(banner("freq", format!("{:.1}", dial::snap(self.freq))).scale(if roomy { Scale::X2 } else { Scale::X1 }).color(hsla(ink)).shadow())
                    .child(div().display(Scale::X2, window).text_color(hsla(p.fg_dim)).child("FM")),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_end()
                    .gap_1()
                    .min_w_0()
                    .child(div().display(Scale::X2, window).text_color(hsla(ink)).child(decrypt(("station", self.tuned), name)))
                    .child(div().body(text::SM).text_color(hsla(p.fg_dim)).child(genre)),
            );

        let [marks, needle] = dial::scale(self.freq, &self.band, scale_cols);
        // The scale's characters are half a display cell wide; the tuning
        // slider's track spans exactly the scale, so the needle and the thumb
        // line up.
        let cols_px = display_size(Scale::X1, window) / 2. * scale_cols as f32;
        let scale = div()
            .flex()
            .flex_col()
            .display(Scale::X1, window)
            .whitespace_nowrap()
            .child(div().text_color(hsla(p.fg_dim)).child(marks))
            .child(div().text_color(hsla(if on { p.accent } else { p.fg_faint })).child(needle));

        let tune = slider("tune")
            .range(dial::LOW, dial::HIGH)
            .step(dial::STEP)
            .value(self.freq)
            .width(cols_px)
            .format(|v| format!("{v:.1} MHz").into())
            .on_change(cx.listener(|this, v: &f32, _, cx| {
                this.sweep_task = None;
                this.set_freq(*v, cx);
            }));

        let controls = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .child(
                Button::new("seek-down")
                    .icon(Icon::ChevronLeft)
                    .secondary()
                    .tooltip("Seek down · Left")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.seek(false, cx))),
            )
            .child(
                Button::new("power")
                    .label(if on { "Off" } else { "On" })
                    .icon(if on { Icon::Stop } else { Icon::Play })
                    .primary()
                    .shortcut("Space")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.power(!this.on, cx))),
            )
            .child(
                Button::new("seek-up")
                    .icon(Icon::ChevronRight)
                    .secondary()
                    .tooltip("Seek up · Right")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.seek(true, cx))),
            )
            .child(div().flex_1())
            .child(div().display(Scale::X1, window).text_color(hsla(p.fg_dim)).child("VOL"))
            .child(
                slider("volume")
                    .range(0., 1.)
                    .step(0.05)
                    .value(self.settings.volume)
                    .width(px(200.))
                    .format(|v| format!("{:.0}%", v * 100.).into())
                    .on_change(cx.listener(|this, v: &f32, _, cx| this.set_volume(*v, cx))),
            );

        let lock_meta = match (&status, t.locked()) {
            _ if !on => "off",
            (_, None) => "searching",
            (Status::Playing, Some(_)) => "locked",
            (Status::Failed(_), Some(_)) => "no signal",
            _ => "tuning",
        };
        let tuner = panel("Tuner").meta(lock_meta).child(div().flex().flex_col().gap_4().p_2().child(readout).child(div().flex().flex_col().gap_2().child(scale).child(tune)).child(controls));

        let meter_cells = (scale_cols * 3 / 4).clamp(32, 160);
        let meters = panel("Level").meta(if on { "live" } else { "" }).child(
            div()
                .flex()
                .flex_col()
                .gap_3()
                .p_2()
                .child(meter(self.vu[0]).label("L").id("vu-l").segments(meter_cells as u32).thresholds(0.7, 0.9))
                .child(meter(self.vu[1]).label("R").id("vu-r").segments(meter_cells as u32).thresholds(0.7, 0.9))
                .child(ascii_gauge(if on { t.signal } else { 0. }).label("rf").cells(meter_cells)),
        );

        let problem = self.radio.output_error().or(match &status {
            Status::Failed(why) if on => Some(why.clone()),
            _ => None,
        });

        let gear = Button::new("open-settings").icon(Icon::Sliders).ghost().small().tooltip("Settings · Ctrl+,").on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                this.settings_open = !this.settings_open;
                cx.notify();
            },
        ));
        let drawer = self.settings_drawer(window, cx);

        window_frame().child(power_on_in(
            "power",
            div()
                .flex()
                .flex_col()
                .size_full()
                .bg(hsla(p.bg))
                .text_color(hsla(p.fg))
                .body(text::BASE)
                .on_action(cx.listener(|this, _: &TogglePalette, window, cx| this.palette.update(cx, |p, cx| p.toggle(window, cx))))
                .on_action(cx.listener(|this, _: &OpenSettings, _, cx| {
                    this.settings_open = true;
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &Power, _, cx| this.power(!this.on, cx)))
                .on_action(cx.listener(|this, _: &SeekUp, _, cx| this.seek(true, cx)))
                .on_action(cx.listener(|this, _: &SeekDown, _, cx| this.seek(false, cx)))
                .on_action(cx.listener(|this, _: &VolumeUp, _, cx| this.nudge_volume(0.05, cx)))
                .on_action(cx.listener(|this, _: &VolumeDown, _, cx| this.nudge_volume(-0.05, cx)))
                .child(self.palette.clone())
                .child(self.toaster.clone())
                .child(title_bar("Wireless").child(gear))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h_0()
                        .gap(space::ROW)
                        .p(space::ROW)
                        // Spare height goes around the set, not under it.
                        .justify_center()
                        .child(tuner)
                        .child(meters)
                        .child(div().flex().justify_center().child(marquee("ticker", self.ticker()).cells(scale_cols).color(hsla(p.fg_dim))))
                        .when_some(problem, |col, why| {
                            col.child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .justify_center()
                                    .gap_2()
                                    .body(text::SM)
                                    .text_color(hsla(p.fg_dim))
                                    .child(icon(Icon::Warning).fit(px(14.)).color(hsla(p.warning)))
                                    .child(why),
                            )
                        }),
                )
                .child(drawer)
                .child(
                    status_bar()
                        .left(if on { "ON" } else { "OFF" })
                        .left(format!("VOL {:.0}%", self.settings.volume * 100.))
                        .right(theme::scheme(cx).name.to_uppercase())
                        .right_live(format!("{}FPS", motion::fps())),
                ),
        ))
    }
}

gpui::actions!(wireless, [OpenSettings, Power, SeekUp, SeekDown, VolumeUp, VolumeDown]);

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        ferrite_design::init(Appearance::Dark, cx);
        cx.bind_keys([
            KeyBinding::new("ctrl-shift-p", TogglePalette, None),
            KeyBinding::new("ctrl-,", OpenSettings, None),
            KeyBinding::new("space", Power, None),
            KeyBinding::new("right", SeekUp, None),
            KeyBinding::new("left", SeekDown, None),
            KeyBinding::new("up", VolumeUp, None),
            KeyBinding::new("down", VolumeDown, None),
        ]);
        // Wide enough for the dial's narrowest face at 150%.
        let options = chrome::remembered_window_options("wireless", "Wireless", size(px(1180.), px(620.)), cx);
        cx.open_window(options, |window, cx| {
            chrome::square_corners(window);
            chrome::remember_window("wireless", window, cx);
            chrome::power_off_on_close(window, cx);
            cx.new(|cx| Wireless::new(window, cx))
        })
        .expect("failed to open the window");
        cx.activate(true);
    });
}
