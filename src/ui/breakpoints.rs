//! The breakpoint manager of the Breakpoints window, after cpclib's VS Code one (ACE's): a tab
//! for each kind (execution, memory, I/O port, raster), a list of predefined ones, and the form
//! cpclib-dap's `ManagerBreakpoint` reads. What it makes is added through the commands.

use serde_json::{json, Value};

use crate::commands::{Arg, Command, CommandBus};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Exec,
    Memory,
    Io,
    Raster,
}

impl Tab {
    const ALL: [(Tab, &'static str); 4] = [
        (Tab::Exec, "Execution"),
        (Tab::Memory, "Memory"),
        (Tab::Io, "Ports"),
        (Tab::Raster, "Raster"),
    ];

    fn kind(self) -> &'static str {
        match self {
            Tab::Exec => "exec",
            Tab::Memory => "mem",
            Tab::Io => "io",
            Tab::Raster => "raster",
        }
    }
}

/// A predefined breakpoint: its name and the form fields it fills.
type Preset = (&'static str, &'static [(&'static str, &'static str)]);

/// The VS Code manager's presets.
const EXEC: &[Preset] = &[
    ("Interrupt (IM 1 handler, &0038)", &[("address", "&0038")]),
    ("Firmware: TXT OUTPUT (&BB5A)", &[("address", "&BB5A")]),
    ("Firmware: KM WAIT KEY (&BB18)", &[("address", "&BB18")]),
    ("Firmware: SCR SET MODE (&BC0E)", &[("address", "&BC0E")]),
    ("Firmware: MC WAIT FLYBACK (&BD19)", &[("address", "&BD19")]),
    ("Firmware: CAS IN OPEN (&BC77)", &[("address", "&BC77")]),
    (
        "AMSDOS: BIOS READ SECTOR (&C03C, ROM 7)",
        &[
            ("address", "&C03C"),
            ("condition", "UpperROMOn AND UpperROMNum = 7"),
            ("name", "BIOS READ SECTOR"),
        ],
    ),
];
const MEMORY: &[Preset] = &[
    (
        "Screen memory written (&C000-&FFFF)",
        &[("address", "&C000"), ("size", "16384"), ("access", "w")],
    ),
    (
        "Firmware jumpblock written (&BB00-&BDFF)",
        &[("address", "&BB00"), ("size", "768"), ("access", "w")],
    ),
    (
        "Restarts written (&0000-&003F)",
        &[("address", "&0000"), ("size", "64"), ("access", "w")],
    ),
];
const IO: &[Preset] = &[
    (
        "Gate Array: any write",
        &[("address", "&4000"), ("mask", "&C000"), ("access", "w")],
    ),
    (
        "Gate Array: pen select",
        &[
            ("address", "&4000"),
            ("mask", "&C000"),
            ("access", "w"),
            ("value", "&00"),
            ("valmask", "&C0"),
        ],
    ),
    (
        "Gate Array: ink (colour)",
        &[
            ("address", "&4000"),
            ("mask", "&C000"),
            ("access", "w"),
            ("value", "&40"),
            ("valmask", "&C0"),
        ],
    ),
    (
        "Gate Array: mode and ROMs (RMR)",
        &[
            ("address", "&4000"),
            ("mask", "&C000"),
            ("access", "w"),
            ("value", "&80"),
            ("valmask", "&C0"),
        ],
    ),
    (
        "RAM banking (MMR)",
        &[
            ("address", "&4000"),
            ("mask", "&C000"),
            ("access", "w"),
            ("value", "&C0"),
            ("valmask", "&C0"),
        ],
    ),
    (
        "CRTC: register select",
        &[("address", "&0000"), ("mask", "&4300"), ("access", "w")],
    ),
    (
        "CRTC: register write",
        &[("address", "&0100"), ("mask", "&4300"), ("access", "w")],
    ),
    (
        "Upper ROM select",
        &[("address", "&0000"), ("mask", "&2000"), ("access", "w")],
    ),
    (
        "PPI port A (PSG data)",
        &[("address", "&0000"), ("mask", "&0B00")],
    ),
    (
        "PPI port C (PSG control, keyboard line)",
        &[("address", "&0200"), ("mask", "&0B00"), ("access", "w")],
    ),
    (
        "PPI control",
        &[("address", "&0300"), ("mask", "&0B00"), ("access", "w")],
    ),
    (
        "FDC: data register",
        &[("address", "&0101"), ("mask", "&0581")],
    ),
    (
        "FDC: status read",
        &[("address", "&0100"), ("mask", "&0581"), ("access", "r")],
    ),
    (
        "FDC: motor",
        &[("address", "&0000"), ("mask", "&0581"), ("access", "w")],
    ),
];
const RASTER: &[Preset] = &[
    (
        "Frame start (line 0, NOP 0)",
        &[("line", "0"), ("nop", "0")],
    ),
    (
        "Middle of the frame (line 156)",
        &[("line", "156"), ("nop", "0")],
    ),
];

/// The manager's form.
pub struct Manager {
    tab: Tab,
    address: String,
    mask: String,
    access: usize,
    size: String,
    value: String,
    valmask: String,
    condition: String,
    watch: bool,
    step: String,
    name: String,
    line: String,
    nop: String,
    message: Option<(bool, String)>,
}

impl Default for Manager {
    fn default() -> Self {
        Self {
            tab: Tab::Exec,
            address: String::new(),
            mask: String::new(),
            access: 0,
            size: String::new(),
            value: String::new(),
            valmask: String::new(),
            condition: String::new(),
            watch: false,
            step: String::new(),
            name: String::new(),
            line: String::new(),
            nop: String::new(),
            message: None,
        }
    }
}

const ACCESSES: [(&str, &str); 3] = [
    ("rw", "Read and write"),
    ("r", "Read only"),
    ("w", "Write only"),
];

impl Manager {
    fn clear(&mut self) {
        *self = Manager {
            tab: self.tab,
            ..Manager::default()
        };
    }

    fn apply(&mut self, preset: &Preset) {
        self.clear();
        for (key, value) in preset.1 {
            let value = (*value).to_owned();
            match *key {
                "address" => self.address = value,
                "mask" => self.mask = value,
                "size" => self.size = value,
                "value" => self.value = value,
                "valmask" => self.valmask = value,
                "condition" => self.condition = value,
                "name" => self.name = value,
                "line" => self.line = value,
                "nop" => self.nop = value,
                "access" => {
                    self.access = ACCESSES.iter().position(|(a, _)| *a == value).unwrap_or(0)
                }
                _ => {}
            }
        }
    }

    /// The form as cpclib-dap's manager reads it.
    fn form(&self) -> Value {
        let mut form = json!({ "kind": self.tab.kind() });
        let mut put = |key: &str, text: &str| {
            if !text.trim().is_empty() {
                form[key] = json!(text.trim());
            }
        };
        put("address", &self.address);
        put("mask", &self.mask);
        put("size", &self.size);
        put("value", &self.value);
        put("valmask", &self.valmask);
        put("condition", &self.condition);
        put("step", &self.step);
        put("name", &self.name);
        put("line", &self.line);
        put("nop", &self.nop);
        form["access"] = json!(ACCESSES[self.access].0);
        form["mode"] = json!(if self.watch { "watch" } else { "stop" });
        form
    }

    /// The command adding the breakpoint of the form, or why it cannot.
    fn command(&self, resolve: &dyn Fn(&str) -> Option<u16>) -> Result<Command, String> {
        let breakpoint = cpclib_dap::breakpoint_manager::ManagerBreakpoint::from_form(
            &self.form(),
            0,
            |name| resolve(name).map(u32::from),
        )?;
        let command = match self.tab {
            Tab::Raster if breakpoint.every_line() => {
                return Err("a raster breakpoint here needs its line".to_owned())
            }
            Tab::Raster => Command::with_arg(
                "breakpoint-add-raster",
                Arg::Text(format!("{},{}", breakpoint.address, breakpoint.nop)),
            ),
            // a plain execution one: the PC breakpoint
            Tab::Exec if breakpoint.is_plain_exec() => {
                Command::with_arg("breakpoint-add", Arg::Number(i64::from(breakpoint.address)))
            }
            _ => {
                let label = self.name.trim();
                let mut body = breakpoint.body(label);
                if !label.is_empty() {
                    body["name"] = json!(label);
                }
                Command::with_arg("breakpoint-add-advanced", Arg::Text(body.to_string()))
            }
        };
        command.map_err(|e| e.to_string())
    }
}

/// A memory / I/O breakpoint as `/api/breakpoints` lists it, in a line: `io w &4000/&C000 =&40/&C0
/// stop ×3 "ink"`.
pub fn summary(breakpoint: &Value) -> String {
    let number = |key: &str| breakpoint[key].as_u64().unwrap_or(0);
    let mut text = format!(
        "{} {} &{:04X}",
        breakpoint["kind"].as_str().unwrap_or("?"),
        breakpoint["access"].as_str().unwrap_or(""),
        number("addr")
    );
    if number("mask") != 0xffff {
        text.push_str(&format!("/&{:04X}", number("mask")));
    }
    if number("size") > 1 {
        text.push_str(&format!(" +{}", number("size")));
    }
    if number("valmask") != 0 {
        text.push_str(&format!(
            " =&{:02X}/&{:02X}",
            number("value"),
            number("valmask")
        ));
    }
    if let Some(condition) = breakpoint["condition"].as_str() {
        text.push_str(&format!(" if {condition}"));
    }
    text.push_str(&format!(
        " {}",
        breakpoint["run"].as_str().unwrap_or("stop")
    ));
    if number("hits") > 0 {
        text.push_str(&format!(" hit {}", number("hits")));
    }
    if let Some(name) = breakpoint["name"].as_str() {
        text.push_str(&format!(" \"{name}\""));
    }
    text
}

fn field(ui: &mut egui::Ui, label: &str, text: &mut String, hint: &str) {
    ui.label(label);
    ui.add(
        egui::TextEdit::singleline(text)
            .hint_text(hint)
            .desired_width(150.0),
    );
    ui.end_row();
}

/// The manager: the kind's tab, its predefined ones, the form, Add.
pub fn show(
    ui: &mut egui::Ui,
    manager: &mut Manager,
    bus: &mut CommandBus,
    resolve: &dyn Fn(&str) -> Option<u16>,
) {
    ui.horizontal(|ui| {
        for (tab, label) in Tab::ALL {
            if ui.selectable_label(manager.tab == tab, label).clicked() && manager.tab != tab {
                manager.tab = tab;
                manager.clear();
            }
        }
    });
    let presets = match manager.tab {
        Tab::Exec => EXEC,
        Tab::Memory => MEMORY,
        Tab::Io => IO,
        Tab::Raster => RASTER,
    };
    egui::Grid::new("manager")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.label("Predefined");
            egui::ComboBox::from_id_salt("preset")
                .selected_text("choose…")
                .width(260.0)
                .show_ui(ui, |ui| {
                    for preset in presets {
                        if ui.selectable_label(false, preset.0).clicked() {
                            manager.apply(preset);
                        }
                    }
                });
            ui.end_row();
            if manager.tab == Tab::Raster {
                field(ui, "Line", &mut manager.line, "0-311");
                field(ui, "NOP", &mut manager.nop, "0-63");
            } else {
                let address = if manager.tab == Tab::Io {
                    "Port"
                } else {
                    "Address"
                };
                field(ui, address, &mut manager.address, "&C03C or a label");
                if manager.tab != Tab::Exec {
                    field(ui, "Mask", &mut manager.mask, "&FFFF");
                    ui.label("Access");
                    egui::ComboBox::from_id_salt("access").show_index(
                        ui,
                        &mut manager.access,
                        ACCESSES.len(),
                        |i| ACCESSES[i].1.to_owned(),
                    );
                    ui.end_row();
                }
                if manager.tab == Tab::Memory {
                    field(ui, "Size", &mut manager.size, "1");
                }
                if manager.tab != Tab::Exec {
                    field(ui, "Value", &mut manager.value, "&00");
                    field(ui, "Value mask", &mut manager.valmask, "&00 (any)");
                }
                field(ui, "Condition", &mut manager.condition, "BC=7 AND FlagC");
                ui.label("Mode");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut manager.watch, false, "Stop");
                    ui.radio_value(&mut manager.watch, true, "Watch (report, go on)");
                });
                ui.end_row();
                field(ui, "Pass", &mut manager.step, "0: at once");
            }
            field(ui, "Name", &mut manager.name, "");
        });
    ui.horizontal(|ui| {
        if ui.button("Add").clicked() {
            manager.message = Some(match manager.command(resolve) {
                Ok(command) => {
                    bus.push(command);
                    (false, "added".to_owned())
                }
                Err(error) => (true, error),
            });
        }
        if ui.button("New").clicked() {
            manager.clear();
        }
        if let Some((error, text)) = &manager.message {
            if *error {
                ui.colored_label(ui.visuals().error_fg_color, text);
            } else {
                ui.weak(text);
            }
        }
    });
}

#[cfg(test)]
mod test {
    use super::*;

    fn resolve(name: &str) -> Option<u16> {
        (name == "main").then_some(0x4000)
    }

    #[test]
    fn the_form_makes_the_command_of_its_kind() {
        let mut manager = Manager::default();
        manager.address = "main".into();
        let command = manager.command(&resolve).unwrap();
        assert_eq!(
            (command.id, command.arg),
            ("breakpoint-add", Some(Arg::Number(0x4000)))
        );

        manager.tab = Tab::Io;
        manager.apply(&IO[2]);
        let command = manager.command(&resolve).unwrap();
        assert_eq!(command.id, "breakpoint-add-advanced");
        let Some(Arg::Text(body)) = command.arg else {
            panic!()
        };
        let body: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            (
                &body["kind"],
                &body["mask"],
                &body["value"],
                &body["valmask"],
                &body["access"]
            ),
            (
                &json!("io"),
                &json!(0xc000),
                &json!(0x40),
                &json!(0xc0),
                &json!("w")
            )
        );

        manager.tab = Tab::Raster;
        manager.apply(&RASTER[1]);
        let command = manager.command(&resolve).unwrap();
        assert_eq!(command.arg, Some(Arg::Text("156,0".into())));

        manager.tab = Tab::Memory;
        manager.clear();
        manager.address = "nowhere".into();
        assert!(manager.command(&resolve).is_err());
    }
}
