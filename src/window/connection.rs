//! Pedal and ToneStrand Box connections: polling the device thread, events, pairing, and the sidebar cards that pick a pedal.

use super::*;

impl App {
    pub(super) fn poll(&mut self, ctx: &egui::Context) {
        if let Some(e) = boxes::take_save_error() {
            self.note(LogLevel::Bad, e);
        }
        if self.usb_scanned.is_none_or(|t| t.elapsed() > Duration::from_secs(3)) {
            self.usb = wire::list_usb_pedals();
            self.usb_scanned = Some(Instant::now());
            // "Follow the computer": pick up a change of its light or dark setting.
            if self.settings.theme == "system" && theme::wants_light("system", ctx) != theme::is_light() {
                theme::set_light(!theme::is_light());
                theme::apply(ctx);
            }
        }
        self.maybe_reconnect(ctx);
        // A box that did not answer (it changed network, or went into direct mode) is looked for again by its .local name every
        // few seconds, so it is found at its new address in seconds, not minutes.
        let unanswered = matches!(&self.status, Some((_, Err(_)))) || (self.boxes.is_empty() && self.settings.discover_boxes);
        if unanswered && self.box_lookup_at.is_none_or(|t| t.elapsed() > Duration::from_secs(5)) {
            self.box_lookup_at = Some(Instant::now());
            boxes::lookup_local_names_once(self.box_tx.clone());
        }
        let mut moved: Option<String> = None;
        while let Ok(b) = self.box_rx.try_recv() {
            if let Some(existing) = self.boxes.iter_mut().find(|x| (!b.id.is_empty() && x.id == b.id) || x.host == b.host) {
                if existing.host != b.host {
                    // The same box at another address: follow it and ask it again.
                    moved = Some(b.host.clone());
                }
                existing.host = b.host.clone();
                if existing.id.is_empty() {
                    existing.id = b.id.clone();
                }
            } else {
                self.note(LogLevel::Info, format!("Found {} at {}", b.name, b.host));
                self.boxes.push(b);
            }
        }
        if let Some(host) = moved {
            self.note(LogLevel::Info, format!("The box is now at {host}"));
            self.check_box(host);
        }
        if let Some((host, rx)) = &self.status_rx {
            if let Ok(r) = rx.try_recv() {
                let host = host.clone();
                // Learn the box id from its status, so its token is kept by id.
                if let Ok(st) = &r {
                    if let Some(b) = self.boxes.iter_mut().find(|b| b.host == host) {
                        if b.id.is_empty() && !st.id.is_empty() {
                            b.id = st.id.clone();
                        }
                    }
                }
                self.status = Some((host, r));
                self.status_rx = None;
            }
        }
        if let Some(p) = &self.pairing {
            if let Ok(r) = p.rx.try_recv() {
                let key = p.key.clone();
                let host = p.host.clone();
                self.pairing = None;
                match r {
                    Ok(token) => {
                        self.settings.tokens.insert(key, token);
                        if let Some(b) = self.boxes.iter().find(|b| b.host == host).cloned() {
                            self.settings.saved.retain(|s| s.host != b.host);
                            self.settings.saved.push(b);
                        }
                        self.settings.save();
                        self.note(LogLevel::Good, "Paired with the box. You can connect now.");
                    }
                    Err(e) => self.note(LogLevel::Bad, format!("Pairing failed: {e}")),
                }
            }
        }

        let mut events = Vec::new();
        if let Some(l) = &self.link {
            while let Ok(e) = l.rx.try_recv() {
                events.push(e);
            }
        }
        for e in events {
            self.on_event(e);
        }
        if self.link.is_some() || self.pairing.is_some() || self.status_rx.is_some() {
            ctx.request_repaint_after(Duration::from_millis(200));
        } else {
            ctx.request_repaint_after(Duration::from_secs(2));
        }
    }

    /// After a USB pedal dropped out: once it has left the USB list and shows up again (it
    /// was unplugged and plugged back in), connect to it once more. Never for a ToneStrand Box,
    /// and never after the person pressed Disconnect.
    /// Make a dropped link again: a few tries, further and further apart. A USB pedal that has
    /// left the USB list is waited for (that costs no tries); a link that dropped while the
    /// pedal is still there, and a ToneStrand Box, are tried again after a short pause.
    fn maybe_reconnect(&mut self, ctx: &egui::Context) {
        if !self.settings.auto_reconnect || self.user_disconnected {
            self.reconnect = None;
            return;
        }
        let Some(r) = self.reconnect.as_mut() else { return };
        if r.trying || self.link.is_some() || Instant::now() < r.next {
            return;
        }
        if let Target::Usb { pid } = &r.target {
            if !self.usb.iter().any(|p| p.pid == *pid) {
                r.waiting = true;
                return;
            }
            if r.waiting {
                r.waiting = false;
                self.note(LogLevel::Info, "The pedal is back; connecting again");
            }
        }
        let Some(r) = self.reconnect.as_mut() else { return };
        r.trying = true;
        let target = r.target.clone();
        self.start_link(target, ctx);
    }

    pub(super) fn on_event(&mut self, e: Event) {
        match e {
            Event::Connected(name) => {
                self.reconnect = None;
                self.last_error = None;
                if !self.settings.hide_getting_started {
                    self.settings.hide_getting_started = true;
                    self.settings.save();
                }
                self.view = View::Editor;
                self.connecting = false;
                self.busy = None;
                self.note(LogLevel::Good, format!("Connected to {name}"));
                // The connected pedal decides the unit.
                if let Some(u) = crate::snapshots::unit_for_product(&name) {
                    if u != self.unit {
                        self.note(LogLevel::Info, format!("Detected {}", crate::units::unit_name(u)));
                        self.set_unit(u);
                    }
                }
                self.connected = Some(name);
                if !self.settings.units_seen.contains(&self.unit) && self.screenshot.is_none() {
                    self.settings.units_seen.push(self.unit);
                    self.settings.save();
                    self.unit_notice = Some(self.unit);
                }
            }
            Event::Failed(why) => {
                self.connecting = false;
                self.busy = None;
                self.link = None;
                self.last_error = Some(why.clone());
                match self.reconnect.as_mut().filter(|r| r.trying) {
                    Some(r) => {
                        r.trying = false;
                        r.tries += 1;
                        if r.tries >= RECONNECT_TRIES {
                            self.reconnect = None;
                            self.note(LogLevel::Bad, format!("Could not connect again: {why}. Gave up; connect by hand."));
                        } else {
                            r.next = Instant::now() + reconnect_delay(r.tries);
                            let (n, wait) = (r.tries, reconnect_delay(r.tries).as_secs());
                            self.note(
                                LogLevel::Info,
                                format!("Could not connect again ({why}); try {} of {RECONNECT_TRIES} in {wait} s", n + 1),
                            );
                        }
                    }
                    None => {
                        self.reconnect = None;
                        self.note(LogLevel::Bad, format!("Could not connect: {why}"));
                    }
                }
            }
            Event::Disconnected(why) => {
                self.busy = None;
                self.link = None;
                self.connected = None;
                self.presets.clear();
                self.setlists.clear();
                self.irs = None;
                self.ir_sel.clear();
                self.ir_kinds.clear();
                self.current = None;
                self.chosen = None;
                for d in &mut self.docs {
                    if let DocKind::Device { origin, .. } = &mut d.kind {
                        if matches!(origin, Origin::Live { .. }) {
                            *origin = Origin::File;
                            d.title = format!("{} (offline)", d.title);
                        }
                    }
                }
                self.note(LogLevel::Info, why);
                if self.settings.auto_reconnect && !self.user_disconnected {
                    if let Some(t) = self.last_target.clone() {
                        self.reconnect = Some(Reconnect::new(t));
                        self.note(LogLevel::Info, "It will try to connect again");
                    }
                }
            }
            Event::Busy(m) => self.busy = Some(m),
            Event::Info(m) => self.note(LogLevel::Info, m),
            Event::Error(m) => {
                self.refresh.failed();
                self.busy = None;
                self.note(LogLevel::Bad, m);
            }
            Event::Done(m) => {
                self.busy = None;
                self.note(LogLevel::Good, m);
            }
            Event::TestStep { name, ok, detail } => self.tests.log.push((name, ok, detail)),
            Event::TestDone { failures } => {
                self.tests.running = false;
                self.tests.done = Some(failures);
            }
            Event::Irs(list) => {
                crate::nam::set_ir_names(&list);
                // Slots not looked at yet (or holding something new) get sorted into IRs and NAM captures.
                let unknown: Vec<(i64, String)> =
                    list.iter().filter(|(s, n)| !self.ir_kinds.contains_key(&(*s, n.clone()))).cloned().collect();
                self.ir_kinds.retain(|k, _| list.contains(k));
                crate::nam::set_slot_kinds(self.ir_kinds.iter().map(|((s, _), n)| (*s, *n)).collect());
                self.irs = Some(list);
                self.ir_sel.retain(|_| true);
                if !unknown.is_empty() {
                    if let Some(l) = &self.link {
                        let _ = l.tx.send(Cmd::IrKinds(unknown));
                    }
                }
            }
            Event::IrKinds(kinds) => {
                self.busy = None;
                for (s, n, nam) in kinds {
                    self.ir_kinds.insert((s, n), nam);
                }
                crate::nam::set_slot_kinds(self.ir_kinds.iter().map(|((s, _), n)| (*s, *n)).collect());
            }
            Event::Setlists(s) => self.setlists = s,
            Event::Presets(sl, p) => {
                self.setlist = sl;
                self.presets = p;
            }
            Event::Current(sl, idx, name) => self.current = Some((sl, idx, name)),
            Event::Document { setlist, index, live, purpose, blob } => {
                // Edits sent in a row each bring a copy of the loaded preset back; only the last one is current.
                // An older copy would put the state from before the newest edit back on screen.
                if live && purpose == Purpose::Edit && !self.refresh.arrived(std::time::Instant::now()) {
                    return;
                }
                self.busy = None;
                let slot = index.max(0) as usize;
                let name = self.presets.get(slot).cloned().unwrap_or_default();
                let Some(blob) = blob else {
                    self.note(LogLevel::Info, format!("{} is empty", slot_label(slot)));
                    return;
                };
                if purpose == Purpose::SaveToFile {
                    let _ = std::fs::create_dir_all(&self.library_dir);
                    let default = format!("{} {}.hxpreset", slot_label(slot), device::sanitize(&name));
                    let id = crate::platform::FileDialog::new()
                        .set_directory(&self.library_dir)
                        .set_file_name(&default)
                        .add_filter("Pedal preset", &["hxpreset"])
                        .save_file();
                    self.start_pick(id, picks::Pick::SavePedalPreset(blob));
                    return;
                }
                match DevicePreset::parse(&blob) {
                    Ok(preset) => {
                        let origin = if live { Origin::Live { setlist, index } } else { Origin::Slot { setlist, index } };
                        let label = slot_label(slot);
                        let title = if live { format!("Live · {label} {name}") } else { format!("{label} {name}") };
                        // There is one live tab: it follows whatever the pedal is playing.
                        let existing =
                            if live { self.docs.iter().position(|d| d.is_live()) } else { self.docs.iter().position(|d| d.title == title) };
                        if let Some(i) = existing {
                            let keep = self.docs[i].selected.filter(|_| self.docs[i].name == name);
                            self.docs[i].kind = DocKind::Device { preset, origin };
                            self.docs[i].dirty = false;
                            self.docs[i].title = title;
                            self.docs[i].name = name;
                            self.docs[i].selected = keep;
                            if live && !self.docs[self.active.min(self.docs.len() - 1)].is_live() && self.view == View::Editor {
                                // Do not pull the person away from another tab on a refresh.
                            } else {
                                self.active = i;
                            }
                        } else {
                            self.docs.push(Doc {
                                id: next_doc_id(),
                                title,
                                name,
                                path: None,
                                kind: DocKind::Device { preset, origin },
                                dirty: false,
                                selected: None,
                                example: false,
                                undo: Vec::new(),
                                redo: Vec::new(),
                                last_edit: None,
                            });
                            self.active = self.docs.len() - 1;
                        }
                    }
                    Err(e) => self.note(LogLevel::Bad, e),
                }
            }
        }
    }

    // --------------------------------------------------------------- actions ---

    pub(super) fn connect(&mut self, ctx: &egui::Context) {
        let Some(pick) = self.pick.clone() else { return };
        let target = match pick {
            Pick::Usb(i) => match self.usb.get(i) {
                Some(p) => Target::Usb { pid: p.pid },
                None => return,
            },
            Pick::Box(i) => {
                let Some(b) = self.boxes.get(i).cloned() else { return };
                let Some(token) = self.settings.token_for(&b) else {
                    self.note(LogLevel::Bad, "Pair with this box first.");
                    return;
                };
                Target::Box { host: b.host.clone(), token, label: b.name.clone() }
            }
        };
        // A connection the person asked for: forget any automatic one pending.
        self.reconnect = None;
        self.user_disconnected = false;
        self.start_link(target, ctx);
    }

    fn start_link(&mut self, target: Target, ctx: &egui::Context) {
        self.last_target = Some(target.clone());
        self.refresh.reset();
        let c = ctx.clone();
        self.link = Some(device::connect(target, move || c.request_repaint()));
        self.connecting = true;
    }

    pub(super) fn disconnect(&mut self) {
        self.user_disconnected = true;
        self.reconnect = None;
        if let Some(l) = &self.link {
            let _ = l.tx.send(Cmd::Disconnect);
        }
    }

    pub(super) fn check_box(&mut self, host: String) {
        let (tx, rx) = channel();
        let h = host.clone();
        std::thread::spawn(move || {
            let _ = tx.send(boxes::relay_status(&h));
        });
        self.status_rx = Some((host, rx));
    }

    pub(super) fn start_pairing(&mut self, b: &BoxInfo) {
        let (tx, rx) = channel();
        let host = b.host.clone();
        std::thread::spawn(move || {
            let r = (|| {
                let nonce = boxes::pair_start(&host)?;
                let until = Instant::now() + Duration::from_secs(boxes::PAIR_WINDOW_SECS + 2);
                while Instant::now() < until {
                    std::thread::sleep(Duration::from_millis(800));
                    match boxes::pair_poll(&host, &nonce)? {
                        PairPoll::Token(t) => return Ok(t),
                        PairPoll::Waiting(_) => {}
                        PairPoll::Closed => return Err("the pairing window closed before the button was pressed".to_string()),
                    }
                }
                Err("the button on the box was not pressed in time".to_string())
            })();
            let _ = tx.send(r);
        });
        self.pairing = Some(Pairing { host: b.host.clone(), key: boxes::box_key(b), rx, started: Instant::now() });
    }

    pub(super) fn devices_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        side_section(ui, "CONNECT", |ui| {
            ui.add_space(2.0);
            let locked = self.link.is_some();
            // Connected: one line, the rest is in the DEVICE box above.
            if locked {
                // Where the link goes: the cable, or the box and which network it is on (its own, in direct mode, has this fixed address).
                match &self.last_target {
                    Some(crate::device::Target::Box { host, label, .. }) => {
                        ui.label(RichText::new(format!("{label}  ·  {host}")).small().color(muted()));
                        let own = host == "192.168.4.1";
                        ui.label(
                            RichText::new(if own { "Direct: the box's own network, no router" } else { "Through your Wi-Fi" })
                                .small()
                                .color(if own { good() } else { muted() }),
                        );
                    }
                    Some(crate::device::Target::Usb { .. }) => {
                        ui.label(RichText::new("USB cable").small().color(muted()));
                    }
                    None => {}
                }
                ui.horizontal(|ui| {
                    if ui.button("Disconnect").clicked() {
                        self.disconnect();
                        ui.close_menu();
                    }
                    if self.connected.is_some() && ui.button("Refresh").on_hover_text("Read the preset list again").clicked() {
                        self.send(Cmd::Refresh);
                        ui.close_menu();
                    }
                });
                return;
            }
            if !self.usb.is_empty() {
                ui.label(RichText::new("USB").small().color(muted()));
            }
            for (i, p) in self.usb.clone().iter().enumerate() {
                let sel = self.pick == Some(Pick::Usb(i));
                if ui.add_enabled(!locked, egui::SelectableLabel::new(sel, format!("🎸  {}", p.product))).clicked() {
                    self.pick = Some(Pick::Usb(i));
                    self.pick_manual = true;
                }
            }
            ui.add_space(4.0);
            ui.label(RichText::new("ToneStrand Box (Wi-Fi)").small().color(muted()));
            if self.boxes.is_empty() {
                ui.label(RichText::new("Looking for boxes on your network...").color(muted()).italics());
            }
            for (i, b) in self.boxes.clone().iter().enumerate() {
                let sel = self.pick == Some(Pick::Box(i));
                let paired = self.settings.token_for(b).is_some();
                ui.horizontal(|ui| {
                    let text = format!("📡  {}", if b.name.is_empty() { &b.host } else { &b.name });
                    if ui.add_enabled(!locked, egui::SelectableLabel::new(sel, text)).clicked() {
                        self.pick = Some(Pick::Box(i));
                        self.pick_manual = true;
                        self.check_box(b.host.clone());
                    }
                    if paired {
                        theme::pill(ui, "paired", good());
                    }
                });
            }
            // Adding a box by its address: tucked behind a small link.
            let addr_id = egui::Id::new("show-box-address");
            let show_addr = ui.data(|d| d.get_temp::<bool>(addr_id)).unwrap_or(false) || self.boxes.is_empty();
            if !show_addr && ui.small_button("+ Add a box by address").clicked() {
                ui.data_mut(|d| d.insert_temp(addr_id, true));
            }
            if show_addr {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.add_box_text).hint_text("box address, e.g. 192.168.1.40").desired_width(170.0),
                    );
                    if ui.button("Add").clicked() && !self.add_box_text.trim().is_empty() {
                        let host = self.add_box_text.trim().to_owned();
                        if !self.boxes.iter().any(|b| b.host == host) {
                            self.boxes.push(BoxInfo { name: format!("Box at {host}"), host: host.clone(), id: String::new() });
                        }
                        if let Some(i) = self.boxes.iter().position(|b| b.host == host) {
                            self.pick = Some(Pick::Box(i));
                        }
                        self.check_box(host);
                        self.add_box_text.clear();
                    }
                });
            }

            if let Some(Pick::Box(i)) = self.pick.clone() {
                if let Some(b) = self.boxes.get(i).cloned() {
                    ui.add_space(4.0);
                    // The box's details (pedal attached, relay mode, pairing) folded away.
                    let paired = self.settings.token_for(&b).is_some();
                    egui::CollapsingHeader::new(RichText::new("Box details").small())
                        .id_salt(("box-details", b.host.clone()))
                        .default_open(!paired)
                        .show(ui, |ui| self.box_details(ui, &b, locked));
                }
            }

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if self.link.is_some() {
                    if ui.button("Disconnect").clicked() {
                        self.disconnect();
                    }
                    if self.connected.is_some() && ui.button("Refresh").clicked() {
                        self.send(Cmd::Refresh);
                    }
                } else {
                    let can = match &self.pick {
                        Some(Pick::Usb(_)) => true,
                        Some(Pick::Box(i)) => self.boxes.get(*i).is_some_and(|b| self.settings.token_for(b).is_some()),
                        None => false,
                    };
                    let btn = egui::Button::new(RichText::new("Connect").strong().color(on_accent())).fill(accent_fill());
                    if ui.add_enabled(can, btn).on_disabled_hover_text("Pick a pedal, or a paired box, first").clicked() {
                        ui.close_menu();
                        self.connect(ctx);
                    }
                }
            });
            if let Some(Pick::Box(i)) = &self.pick {
                if self.link.is_none() && self.boxes.get(*i).is_some_and(|b| self.settings.token_for(b).is_none()) {
                    ui.label(
                        RichText::new("This computer is not paired with the box yet: press Pair above, then the BOOT button on the box.")
                            .small()
                            .color(orange()),
                    );
                }
            }
            if let Some(e) = &self.last_error {
                ui.label(RichText::new(format!("Last try: {e}")).small().color(bad()));
            }
        });
    }

    pub(super) fn box_details(&mut self, ui: &mut egui::Ui, b: &BoxInfo, locked: bool) {
        egui::Frame::new().fill(card_hi()).corner_radius(CornerRadius::same(8)).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(format!("{}  ·  {}", b.name, b.host)).strong());
            let status = self.status.clone();
            match &status {
                Some((h, Ok(st))) if *h == b.host => {
                    ui.label(match &st.pedal {
                        Some(p) => RichText::new(format!("Pedal attached: {p}")).color(good()),
                        None => RichText::new("No pedal attached to the box").color(orange()),
                    });
                    if st.direct {
                        ui.label(RichText::new("Direct mode: the box is on its own Wi-Fi network (until it restarts)").color(good()));
                    }
                    ui.label(RichText::new(if st.relay_only { "Relay mode: on" } else { "Relay mode: off" }).color(if st.relay_only {
                        good()
                    } else {
                        orange()
                    }));
                    if st.client && !locked {
                        ui.label(RichText::new("Another computer is using it right now").color(orange()));
                    }
                    if !st.relay_only {
                        if let Some(tok) = self.settings.token_for(b) {
                            if ui.button("Turn relay mode on").clicked() {
                                match boxes::set_relay(&b.host, &tok, true) {
                                    Ok(_) => self.note(LogLevel::Good, "Relay mode is on."),
                                    Err(e) => self.note(LogLevel::Bad, e),
                                }
                                self.check_box(b.host.clone());
                            }
                        }
                    }
                }
                Some((h, Err(e))) if *h == b.host => {
                    ui.label(RichText::new(e).color(bad()));
                }
                _ => {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(12.0));
                        ui.label(RichText::new("Asking the box...").color(muted()));
                    });
                }
            }
            ui.horizontal_wrapped(|ui| {
                if ui.small_button("Check again").clicked() {
                    self.check_box(b.host.clone());
                    // Also look for it by name: it may have moved to another network (direct mode, or back to Wi-Fi).
                    self.box_lookup_at = Some(Instant::now());
                    boxes::lookup_local_names_once(self.box_tx.clone());
                }
                if let Some(p) = &self.pairing {
                    let left = boxes::PAIR_WINDOW_SECS.saturating_sub(p.started.elapsed().as_secs());
                    ui.add(egui::Spinner::new().size(12.0).color(orange()));
                    ui.label(RichText::new(format!("Press BOOT on the box now ({left}s)")).color(orange()).strong());
                } else if ui
                    .small_button(if self.settings.token_for(b).is_some() { "Pair again" } else { "Pair" })
                    .on_hover_text("Then press the BOOT button on the box once")
                    .clicked()
                {
                    self.start_pairing(b);
                }
                if self.settings.saved.iter().any(|s| s.host == b.host) && ui.small_button("Forget").clicked() {
                    self.settings.saved.retain(|s| s.host != b.host);
                    self.settings.tokens.remove(&boxes::box_key(b));
                    self.settings.save();
                }
            });
        });
    }

    /// A preset was dropped on another slot of the pedal's list: copy, swap or move.
    pub(super) fn slot_drop_dialog(&mut self, ctx: &egui::Context) {
        use crate::device::SlotOp;
        let Some((from, to)) = self.slot_drop else { return };
        let empty = |i: usize| self.presets.get(i).is_none_or(|n| n.trim().is_empty());
        let name = |i: usize| if empty(i) { "<empty>".to_owned() } else { self.presets[i].clone() };
        if empty(from) || from == to {
            self.slot_drop = None;
            return;
        }
        let target_empty = empty(to);
        // None: still asking. Some(None): cancelled. Some(Some(op)): go.
        let mut choice: Option<Option<SlotOp>> = None;
        let (lf, lt) = (slot_label(from), slot_label(to));
        egui::Window::new("Copy, swap or move").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(
            ctx,
            |ui| {
                ui.label(RichText::new(format!("{lf}  \"{}\"   onto   {lt}  \"{}\"", name(from), name(to))).strong());
                ui.add_space(2.0);
                ui.label(
                    RichText::new(if target_empty {
                        format!("{lt} is empty. Copy keeps {lf} as it is; Move empties it. Whatever changes is backed up first.")
                    } else {
                        format!("{lt} is backed up before it is replaced (and {lf} too, if you swap).")
                    })
                    .small()
                    .color(muted()),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if theme::primary_button(ui, &format!("Copy to {lt}")).on_hover_text(format!("{lf} stays as it is")).clicked() {
                        choice = Some(Some(SlotOp::Copy));
                    }
                    if target_empty {
                        if ui.button(format!("Move to {lt}")).on_hover_text(format!("{lf} is emptied")).clicked() {
                            choice = Some(Some(SlotOp::Move));
                        }
                    } else if ui.button(format!("Swap with {lt}")).on_hover_text("The two presets change places").clicked() {
                        choice = Some(Some(SlotOp::Swap));
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(None);
                    }
                });
            },
        );
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            choice = Some(None);
        }
        match choice {
            Some(Some(op)) => {
                self.slot_drop = None;
                let (setlist, backup) = (self.setlist, self.backup_dir());
                self.send(Cmd::SlotOp { setlist, from: from as i64, to: to as i64, op, backup });
            }
            Some(None) => self.slot_drop = None,
            None => {}
        }
    }

    pub(super) fn pedal_card(&mut self, ui: &mut egui::Ui) {
        side_section(ui, "PRESETS", |ui| {
            if self.connected.is_none() {
                self.connect_prompt(ui);
                return;
            }
            if !self.setlists.is_empty() {
                let current = self.setlists.get(self.setlist as usize).cloned().unwrap_or_default();
                egui::ComboBox::from_id_salt("setlist").selected_text(current).width(ui.available_width() - 8.0).show_ui(ui, |ui| {
                    for (i, s) in self.setlists.clone().iter().enumerate() {
                        if ui.selectable_label(i as i64 == self.setlist, s).clicked() {
                            self.setlist = i as i64;
                            self.chosen = None;
                            self.send(Cmd::Presets(i as i64));
                        }
                    }
                });
            }
            ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("🔍 Filter presets").desired_width(f32::INFINITY));
            ui.horizontal_wrapped(|ui| {
                ui.menu_button("Backups", |ui| {
                    if ui.button("Back up this setlist...").on_hover_text("Save every preset of this setlist as files").clicked() {
                        ui.close_menu();
                        if cfg!(target_os = "android") {
                            self.back_up_setlist_here();
                        } else {
                            let id = crate::platform::FileDialog::new().set_directory(&self.library_dir).pick_folder();
                            self.start_pick(id, picks::Pick::BackupAllFolder(self.setlist, self.presets.len(), self.presets.clone()));
                        }
                    }
                    if ui
                        .button("Back up the whole pedal...")
                        .on_hover_text("Every setlist and all User IRs, into a new folder inside the one you pick")
                        .clicked()
                    {
                        ui.close_menu();
                        if cfg!(target_os = "android") {
                            self.back_up_pedal_here();
                        } else {
                            let id = crate::platform::FileDialog::new().set_directory(&self.library_dir).pick_folder();
                            self.start_pick(id, picks::Pick::FullBackupFolder);
                        }
                    }
                    ui.separator();
                    if ui.button("Restore a backed-up preset...").on_hover_text("Write a backed-up preset back into a slot").clicked() {
                        ui.close_menu();
                        self.open_restore();
                    }
                    if ui
                        .button("Restore the whole pedal...")
                        .on_hover_text("Write back a folder made by \"Back up the whole pedal\": every preset and IR in it, after showing what it holds")
                        .clicked()
                    {
                        ui.close_menu();
                        self.open_restore_all();
                    }
                });
                theme::help(
                    ui,
                    if theme::touch() {
                        "Press, hold and move a preset onto another slot to copy, swap or move it; press and hold without moving for more. Whatever it replaces is backed up first."
                    } else {
                        "Drag a preset onto another slot to copy, swap or move it. Whatever it replaces is backed up first."
                    },
                );
            });
            ui.add_space(4.0);
            let current_idx = self.current.as_ref().filter(|c| c.0 == self.setlist).map(|c| c.1);
            let filter = self.filter.to_lowercase();
            let row_h = 24.0;
            let presets = self.presets.clone();
            let sl = self.setlist;
            // Leave room under the list for the buttons of the picked preset (their height as
            // last drawn: one row, or two when the panel is narrow).
            let actions_id = egui::Id::new("preset-actions-height");
            let reserve = if self.chosen.is_some() { ui.data(|d| d.get_temp::<f32>(actions_id)).unwrap_or(40.0) + 8.0 } else { 0.0 };
            egui::ScrollArea::vertical()
                .id_salt("presets")
                .max_height((ui.available_height() - 10.0 - reserve).max(160.0))
                .auto_shrink([false, true])
                .drag_to_scroll(!crate::theme::carrying(ui.ctx()))
                .show(ui, |ui| {
                    for (i, name) in presets.iter().enumerate() {
                        if !filter.is_empty() && !name.to_lowercase().contains(&filter) {
                            continue;
                        }
                        let is_current = current_idx == Some(i as i64);
                        let is_chosen = self.chosen == Some(i);
                        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), row_h), crate::theme::list_row_sense());
                        crate::theme::focus_ring(ui, &resp, 5);
                        resp.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Button,
                                true,
                                format!("{} {name}{}", slot_label(i), if is_current { ", loaded" } else { "" }),
                            )
                        });
                        let fill = if is_chosen {
                            accent().linear_multiply(0.22)
                        } else if resp.hovered() {
                            card_hi()
                        } else {
                            Color32::TRANSPARENT
                        };
                        let p = ui.painter();
                        p.rect_filled(rect, CornerRadius::same(5), fill);
                        if is_current {
                            p.rect_filled(egui::Rect::from_min_size(rect.min, Vec2::new(3.0, row_h)), CornerRadius::same(2), accent());
                        }
                        p.text(
                            rect.left_center() + Vec2::new(10.0, 0.0),
                            egui::Align2::LEFT_CENTER,
                            slot_label(i),
                            egui::FontId::monospace(12.5),
                            if is_current { accent() } else { muted() },
                        );
                        let shown = if name.trim().is_empty() { "<empty>" } else { name.as_str() };
                        p.text(
                            rect.left_center() + Vec2::new(52.0, 0.0),
                            egui::Align2::LEFT_CENTER,
                            shown,
                            egui::FontId::proportional(14.0),
                            ink(),
                        );
                        // Drag a preset onto another slot to copy, swap or move it.
                        if !presets[i].trim().is_empty() {
                            crate::theme::row_payload(&resp, PresetDrag(i));
                        }
                        if resp.dnd_hover_payload::<PresetDrag>().is_some_and(|d| d.0 != i) {
                            ui.painter().rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.5_f32, accent()), egui::StrokeKind::Inside);
                        }
                        if let Some(from) = resp.dnd_release_payload::<PresetDrag>() {
                            if from.0 != i {
                                self.slot_drop = Some((from.0, i));
                            }
                        }
                        if resp.clicked() {
                            self.chosen = Some(i);
                        }
                        if resp.double_clicked() {
                            self.send(Cmd::Select(sl, i as i64));
                        }
                        crate::theme::row_menu(&resp, |ui| {
                            if ui.button("Load on the pedal").clicked() {
                                self.send(Cmd::Select(sl, i as i64));
                                ui.close_menu();
                            }
                            if ui.button("Open in editor").clicked() {
                                self.send(Cmd::FetchSlot { setlist: sl, index: i as i64, purpose: Purpose::Edit });
                                ui.close_menu();
                            }
                            if ui.button("Save to a file...").clicked() {
                                self.send(Cmd::FetchSlot { setlist: sl, index: i as i64, purpose: Purpose::SaveToFile });
                                ui.close_menu();
                            }
                            if ui.button("Rename...").clicked() {
                                self.rename = Some((sl, i, name.clone()));
                                ui.close_menu();
                            }
                            ui.separator();
                            if ui.button(RichText::new("Clear slot").color(bad())).clicked() {
                                self.confirm = Some((
                                    format!("Clear {} \"{}\"? This empties the slot on the pedal.", slot_label(i), name),
                                    Cmd::Clear { setlist: sl, index: i as i64 },
                                ));
                                ui.close_menu();
                            }
                        });
                    }
                });
            if let Some(i) = self.chosen {
                ui.add_space(4.0);
                let row = ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(slot_label(i)).monospace().color(accent()));
                    if theme::primary_button(ui, "Load").clicked() {
                        self.send(Cmd::Select(sl, i as i64));
                    }
                    if ui.button("Edit").clicked() {
                        self.send(Cmd::FetchSlot { setlist: sl, index: i as i64, purpose: Purpose::Edit });
                    }
                    if ui.button("Rename").clicked() {
                        self.rename = Some((sl, i, self.presets.get(i).cloned().unwrap_or_default()));
                    }
                    if ui.button("To file").clicked() {
                        self.send(Cmd::FetchSlot { setlist: sl, index: i as i64, purpose: Purpose::SaveToFile });
                    }
                });
                ui.data_mut(|d| d.insert_temp(actions_id, row.response.rect.height()));
            }
        });
    }
}

/// What is being dragged in the pedal's preset list: the slot it came from.
struct PresetDrag(usize);

/// How many times a dropped link is tried again, and how long to wait before each.
const RECONNECT_TRIES: u32 = 5;

pub(super) struct Reconnect {
    target: Target,
    /// Failed tries so far.
    tries: u32,
    next: Instant,
    /// A try is under way.
    trying: bool,
    /// The USB pedal is not in the list; waiting for it to come back.
    waiting: bool,
}

impl Reconnect {
    fn new(target: Target) -> Reconnect {
        Reconnect { target, tries: 0, next: Instant::now() + reconnect_delay(0), trying: false, waiting: false }
    }
}

/// 3 s, 6 s, 12 s, 24 s, 48 s.
fn reconnect_delay(tries: u32) -> Duration {
    Duration::from_secs(3 << tries.min(4))
}

impl App {
    /// The pedal list's place while nothing is connected: say so, and offer the pedals and
    /// paired boxes that are there to connect to with one click.
    pub(super) fn connect_prompt(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if self.link.is_some() {
            ui.label(RichText::new("Connecting...").color(muted()));
            return;
        }
        ui.label(RichText::new("Not connected").strong());
        ui.label(RichText::new("Connect to see and edit the presets on your pedal.").color(muted()));
        ui.add_space(6.0);
        let mut any = false;
        for (i, p) in self.usb.clone().iter().enumerate() {
            any = true;
            if theme::primary_button(ui, &format!("Connect to {} (USB)", p.product)).clicked() {
                self.pick = Some(Pick::Usb(i));
                self.pick_manual = true;
                self.connect(&ctx);
            }
        }
        for (i, b) in self.boxes.clone().iter().enumerate() {
            any = true;
            if self.settings.token_for(b).is_some() {
                if theme::primary_button(ui, &format!("Connect to {} (Wi-Fi)", b.name)).clicked() {
                    self.pick = Some(Pick::Box(i));
                    self.pick_manual = true;
                    self.connect(&ctx);
                }
            } else {
                ui.label(
                    RichText::new(format!("{} is on the network: pair with it from the pedal menu at the top.", b.name))
                        .small()
                        .color(muted()),
                );
            }
        }
        if !any {
            ui.label(
                RichText::new("No pedal found yet. Plug it in by USB, or switch on a ToneStrand Box with its relay mode on.")
                    .small()
                    .color(muted()),
            );
        }
        ui.add_space(4.0);
        ui.label(RichText::new("More options are in the pedal menu at the top.").small().color(muted()));
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnect_waits_longer_each_time_then_gives_up() {
        let secs: Vec<u64> = (0..RECONNECT_TRIES).map(|t| reconnect_delay(t).as_secs()).collect();
        assert_eq!(secs, vec![3, 6, 12, 24, 48]);
        assert_eq!(reconnect_delay(9).as_secs(), 48, "never longer than the last step");
    }
}
