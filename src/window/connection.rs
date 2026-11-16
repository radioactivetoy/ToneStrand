//! Pedal and Helrig Box connections: polling the device thread, events, pairing, and the sidebar cards that pick a pedal.

use super::*;

impl App {
    pub(super) fn poll(&mut self, ctx: &egui::Context) {
        if let Some(e) = boxes::take_save_error() {
            self.note(LogLevel::Bad, e);
        }
        if self.usb_scanned.is_none_or(|t| t.elapsed() > Duration::from_secs(3)) {
            self.usb = wire::list_usb_pedals();
            self.usb_scanned = Some(Instant::now());
            self.maybe_reconnect(ctx);
        }
        while let Ok(b) = self.box_rx.try_recv() {
            if let Some(existing) =
                self.boxes.iter_mut().find(|x| (!b.id.is_empty() && x.id == b.id) || x.host == b.host)
            {
                existing.host = b.host.clone();
                if existing.id.is_empty() {
                    existing.id = b.id.clone();
                }
            } else {
                self.note(LogLevel::Info, format!("Found {} at {}", b.name, b.host));
                self.boxes.push(b);
            }
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
    /// was unplugged and plugged back in), connect to it once more. Never for a Helrig Box,
    /// and never after the person pressed Disconnect.
    fn maybe_reconnect(&mut self, ctx: &egui::Context) {
        let Some((pid, gone)) = self.reconnect else { return };
        if self.link.is_some() || !self.settings.auto_reconnect {
            self.reconnect = None;
            return;
        }
        let present = self.usb.iter().any(|p| p.pid == pid);
        if !present {
            self.reconnect = Some((pid, true));
        } else if gone {
            self.reconnect = None;
            self.note(LogLevel::Info, "The pedal is back; connecting again");
            self.start_link(Target::Usb { pid }, ctx);
        }
    }

    pub(super) fn on_event(&mut self, e: Event) {
        match e {
            Event::Connected(name) => {
                self.last_error = None;
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
            }
            Event::Failed(why) => {
                self.connecting = false;
                self.busy = None;
                self.link = None;
                self.reconnect = None;
                self.last_error = Some(why.clone());
                self.note(LogLevel::Bad, format!("Could not connect: {why}"));
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
                    if let Some(pid) = self.last_usb {
                        self.reconnect = Some((pid, false));
                        self.note(LogLevel::Info, "It will connect again when the pedal is plugged back in");
                    }
                }
            }
            Event::Busy(m) => self.busy = Some(m),
            Event::Info(m) => self.note(LogLevel::Info, m),
            Event::Error(m) => {
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
                let unknown: Vec<(i64, String)> = list.iter().filter(|(s, n)| !self.ir_kinds.contains_key(&(*s, n.clone()))).cloned().collect();
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
                    if let Some(p) = rfd::FileDialog::new()
                        .set_directory(&self.library_dir)
                        .set_file_name(&default)
                        .add_filter("Pedal preset", &["hxpreset"])
                        .save_file()
                    {
                        match crate::files::write_atomic(&p, &blob) {
                            Ok(_) => self.note(LogLevel::Good, format!("Saved {}", p.display())),
                            Err(e) => self.note(LogLevel::Bad, format!("Could not save: {e}")),
                        }
                        self.scan_library();
                    }
                    return;
                }
                match DevicePreset::parse(&blob) {
                    Ok(preset) => {
                        let origin =
                            if live { Origin::Live { setlist, index } } else { Origin::Slot { setlist, index } };
                        let label = slot_label(slot);
                        let title = if live { format!("Live · {label} {name}") } else { format!("{label} {name}") };
                        // There is one live tab: it follows whatever the pedal is playing.
                        let existing = if live {
                            self.docs.iter().position(|d| d.is_live())
                        } else {
                            self.docs.iter().position(|d| d.title == title)
                        };
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
        self.last_usb = match &target {
            Target::Usb { pid } => Some(*pid),
            Target::Box { .. } => None,
        };
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
                        PairPoll::Closed => {
                            return Err("the pairing window closed before the button was pressed".to_string())
                        }
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
                ui.horizontal(|ui| {
                    if ui.button("Disconnect").clicked() {
                        self.disconnect();
                    }
                    if self.connected.is_some() && ui.button("Refresh").on_hover_text("Read the preset list again").clicked() {
                        self.send(Cmd::Refresh);
                    }
                });
                return;
            }
            if !self.usb.is_empty() {
                ui.label(RichText::new("USB").small().color(MUTED));
            }
            for (i, p) in self.usb.clone().iter().enumerate() {
                let sel = self.pick == Some(Pick::Usb(i));
                if ui.add_enabled(!locked, egui::SelectableLabel::new(sel, format!("🎸  {}", p.product))).clicked() {
                    self.pick = Some(Pick::Usb(i));
                    self.pick_manual = true;
                }
            }
            ui.add_space(4.0);
            ui.label(RichText::new("Helrig Box (Wi-Fi)").small().color(MUTED));
            if self.boxes.is_empty() {
                ui.label(RichText::new("Looking for boxes on your network...").color(MUTED).italics());
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
                        theme::pill(ui, "paired", GOOD);
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
                    egui::TextEdit::singleline(&mut self.add_box_text)
                        .hint_text("box address, e.g. 192.168.1.40")
                        .desired_width(170.0),
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
                    let btn = egui::Button::new(RichText::new("Connect").strong().color(Color32::from_rgb(8, 26, 24)))
                        .fill(accent());
                    if ui
                        .add_enabled(can, btn)
                        .on_disabled_hover_text("Pick a pedal, or a paired box, first")
                        .clicked()
                    {
                        self.connect(ctx);
                    }
                }
            });
            if let Some(Pick::Box(i)) = &self.pick {
                if self.link.is_none() && self.boxes.get(*i).is_some_and(|b| self.settings.token_for(b).is_none()) {
                    ui.label(RichText::new("This computer is not paired with the box yet: press Pair above, then the BOOT button on the box.").small().color(ORANGE));
                }
            }
            if let Some(e) = &self.last_error {
                ui.label(RichText::new(format!("Last try: {e}")).small().color(BAD));
            }
        });
    }


    pub(super) fn box_details(&mut self, ui: &mut egui::Ui, b: &BoxInfo, locked: bool) {
        egui::Frame::new()
            .fill(card_hi())
            .corner_radius(CornerRadius::same(8))
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(format!("{}  ·  {}", b.name, b.host)).strong());
                let status = self.status.clone();
                match &status {
                    Some((h, Ok(st))) if *h == b.host => {
                        ui.label(match &st.pedal {
                            Some(p) => RichText::new(format!("Pedal attached: {p}")).color(GOOD),
                            None => RichText::new("No pedal attached to the box").color(ORANGE),
                        });
                        ui.label(
                            RichText::new(if st.relay_only { "Relay mode: on" } else { "Relay mode: off" })
                                .color(if st.relay_only { GOOD } else { ORANGE }),
                        );
                        if st.client && !locked {
                            ui.label(RichText::new("Another computer is using it right now").color(ORANGE));
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
                        ui.label(RichText::new(e).color(BAD));
                    }
                    _ => {
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new().size(12.0));
                            ui.label(RichText::new("Asking the box...").color(MUTED));
                        });
                    }
                }
                ui.horizontal_wrapped(|ui| {
                    if ui.small_button("Check again").clicked() {
                        self.check_box(b.host.clone());
                    }
                    if let Some(p) = &self.pairing {
                        let left = boxes::PAIR_WINDOW_SECS.saturating_sub(p.started.elapsed().as_secs());
                        ui.add(egui::Spinner::new().size(12.0).color(ORANGE));
                        ui.label(RichText::new(format!("Press BOOT on the box now ({left}s)")).color(ORANGE).strong());
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

    pub(super) fn pedal_card(&mut self, ui: &mut egui::Ui) {
        side_section(ui, "PRESETS", |ui| {

            if self.connected.is_none() {
                ui.label(RichText::new("Connect to a pedal to see and edit its presets.").color(MUTED));
                return;
            }
            if !self.setlists.is_empty() {
                let current = self.setlists.get(self.setlist as usize).cloned().unwrap_or_default();
                egui::ComboBox::from_id_salt("setlist")
                    .selected_text(current)
                    .width(ui.available_width() - 8.0)
                    .show_ui(ui, |ui| {
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
                if ui
                    .button("Edit loaded preset (live)")
                    .on_hover_text("Changes are heard at once; Save on pedal keeps them")
                    .clicked()
                {
                    self.send(Cmd::ReadCurrent);
                }
                if ui.button("Back up everything...").on_hover_text("Every setlist and all User IRs, into a new folder inside the one you pick").clicked() {
                    if let Some(folder) = rfd::FileDialog::new().set_directory(&self.library_dir).pick_folder() {
                        self.send(Cmd::FullBackup { folder });
                    }
                }
                if ui.button("Back up all").on_hover_text("Save every preset of this setlist as files").clicked() {
                    if let Some(folder) = rfd::FileDialog::new().set_directory(&self.library_dir).pick_folder() {
                        let names = self.presets.clone();
                        self.send(Cmd::Backup { setlist: self.setlist, count: self.presets.len(), folder, names });
                    }
                }
                if ui.button("Restore...").on_hover_text("Write a backed-up preset back into a slot").clicked() {
                    self.open_restore();
                }
            });
            ui.add_space(4.0);
            let current_idx = self.current.as_ref().filter(|c| c.0 == self.setlist).map(|c| c.1);
            let filter = self.filter.to_lowercase();
            let row_h = 24.0;
            let presets = self.presets.clone();
            let sl = self.setlist;
            egui::ScrollArea::vertical().id_salt("presets").max_height((ui.available_height() - 10.0).max(240.0)).auto_shrink([false, true]).show(ui, |ui| {
                for (i, name) in presets.iter().enumerate() {
                    if !filter.is_empty() && !name.to_lowercase().contains(&filter) {
                        continue;
                    }
                    let is_current = current_idx == Some(i as i64);
                    let is_chosen = self.chosen == Some(i);
                    let (rect, resp) =
                        ui.allocate_exact_size(Vec2::new(ui.available_width(), row_h), egui::Sense::click());
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
                        if is_current { accent() } else { MUTED },
                    );
                    let shown = if name.trim().is_empty() { "<empty>" } else { name.as_str() };
                    p.text(
                        rect.left_center() + Vec2::new(52.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        shown,
                        egui::FontId::proportional(14.0),
                        TEXT,
                    );
                    if resp.clicked() {
                        self.chosen = Some(i);
                    }
                    if resp.double_clicked() {
                        self.send(Cmd::Select(sl, i as i64));
                    }
                    resp.context_menu(|ui| {
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
                        if ui.button(RichText::new("Clear slot").color(BAD)).clicked() {
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
                ui.horizontal_wrapped(|ui| {
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
            }
        });
    }
}
