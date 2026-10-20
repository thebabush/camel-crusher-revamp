use super::*;

#[test]
fn factory_matching_checks_all_values_and_tolerance() {
    for (i, preset) in FACTORY_PRESETS.iter().enumerate() {
        assert_eq!(matched(&preset.values), Some(i));
        for slot in 0..12 {
            let mut values = preset.values;
            values[slot] += 0.00005;
            assert_eq!(matched(&values), Some(i));
            values[slot] += 0.001;
            assert_eq!(matched(&values), None);
        }
    }
}

#[derive(Default)]
struct Recorder(std::sync::Mutex<Vec<(u8, ParamPtr, f32)>>);

impl GuiContext for Recorder {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Clap
    }
    fn request_resize(&self) -> bool {
        true
    }
    unsafe fn raw_begin_set_parameter(&self, param: ParamPtr) {
        self.0.lock().unwrap().push((0, param, 0.0));
    }
    unsafe fn raw_set_parameter_normalized(&self, param: ParamPtr, value: f32) {
        self.0.lock().unwrap().push((1, param, value));
    }
    unsafe fn raw_end_set_parameter(&self, param: ParamPtr) {
        self.0.lock().unwrap().push((2, param, 0.0));
    }
    fn get_state(&self) -> PluginState {
        unreachable!()
    }
    fn set_state(&self, _: PluginState) {
        unreachable!()
    }
}

#[test]
fn preset_application_sends_all_twelve_values_with_gestures() {
    let params = CrusherParams::new();
    let pointers = [
        params.dist_on.as_ptr(),
        params.mech.as_ptr(),
        params.tube.as_ptr(),
        params.filter_on.as_ptr(),
        params.cutoff.as_ptr(),
        params.res.as_ptr(),
        params.comp_on.as_ptr(),
        params.comp_amount.as_ptr(),
        params.phat.as_ptr(),
        params.master_on.as_ptr(),
        params.mix.as_ptr(),
        params.volume.as_ptr(),
    ];
    for (index, preset) in FACTORY_PRESETS.iter().enumerate() {
        let recorder = Recorder::default();
        apply(&ParamSetter::new(&recorder), &params, index);
        let events = recorder.0.lock().unwrap();
        assert_eq!(events.len(), 36);
        let mut seen = [false; 12];
        for gesture in events.as_chunks::<3>().0 {
            assert_eq!([gesture[0].0, gesture[1].0, gesture[2].0], [0, 1, 2]);
            assert_eq!(gesture[0].1, gesture[1].1);
            assert_eq!(gesture[1].1, gesture[2].1);
            let slot = pointers.iter().position(|p| *p == gesture[1].1).unwrap();
            assert!(!seen[slot]);
            seen[slot] = true;
            assert!((gesture[1].2 - preset.values[slot]).abs() < 1e-6);
        }
        assert!(seen.into_iter().all(|v| v));
    }
}

#[test]
fn header_texture_tracks_display_scale() {
    let ctx = egui::Context::default();
    let mut state = State {
        header: None,
        last_preset: 0,
        random: 1,
        drag_value: 0.0,
    };
    assert_eq!(state.header(&ctx).size(), [460, 104]);
    let first = state.header(&ctx).id();
    assert_eq!(state.header(&ctx).id(), first);
    ctx.set_pixels_per_point(2.0);
    let _ = ctx.run(egui::RawInput::default(), |_| {});
    assert_eq!(state.header(&ctx).size(), [920, 208]);
    assert_ne!(state.header(&ctx).id(), first);
}

#[test]
fn reopening_editor_uploads_header_to_each_new_renderer() {
    let mut state = State {
        header: None,
        last_preset: 3,
        random: 1,
        drag_value: 0.0,
    };
    for _ in 0..3 {
        let ctx = egui::Context::default();
        initialize_editor(&ctx, &mut state);
        let mut header_id = None;
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            header_id = Some(state.header(ctx).id());
        });
        let (_, delta) = output
            .textures_delta
            .set
            .iter()
            .find(|(id, _)| Some(*id) == header_id)
            .expect("each reopened renderer must receive the header pixels");
        assert_eq!(delta.image.size(), [460, 104]);
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            assert_eq!(Some(state.header(ctx).id()), header_id);
        });
        assert!(!output
            .textures_delta
            .set
            .iter()
            .any(|(id, _)| Some(*id) == header_id));
        assert_eq!(state.last_preset, 3);
    }
}

#[test]
fn knob_drag_uses_one_gesture_and_shift_is_ten_times_finer() {
    for (shift, expected) in [(false, 0.6), (true, 0.51)] {
        let ctx = egui::Context::default();
        let mut fonts = egui::FontDefinitions::default();
        for name in ["bold", "mono"] {
            fonts.families.insert(
                FontFamily::Name(name.into()),
                fonts.families[&FontFamily::Proportional].clone(),
            );
        }
        ctx.set_fonts(fonts);
        let param = FloatParam::new("Test", 0.5, FloatRange::Linear { min: 0.0, max: 1.0 });
        let recorder = Recorder::default();
        let setter = ParamSetter::new(&recorder);
        let mut drag = 0.0;
        let modifiers = egui::Modifiers {
            shift,
            ..Default::default()
        };
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        };
        for events in [
            vec![egui::Event::PointerMoved(pos2(100.0, 100.0))],
            vec![button(pos2(100.0, 100.0), true)],
            vec![egui::Event::PointerMoved(pos2(100.0, 80.0))],
            vec![button(pos2(100.0, 80.0), false)],
        ] {
            let _ = ctx.run(
                egui::RawInput {
                    events,
                    modifiers,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        knob(ui, &setter, &param, "Test", pos2(100.0, 100.0), &mut drag);
                    });
                },
            );
        }
        let events = recorder.0.lock().unwrap();
        assert_eq!(events.iter().filter(|e| e.0 == 0).count(), 1);
        assert_eq!(events.iter().filter(|e| e.0 == 2).count(), 1);
        assert_eq!(events.first().unwrap().0, 0);
        assert_eq!(events.last().unwrap().0, 2);
        let value = events.iter().rev().find(|e| e.0 == 1).unwrap().2;
        assert!((value - expected).abs() < 1e-6, "{value} != {expected}");
    }
}
