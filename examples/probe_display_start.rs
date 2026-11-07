//! After a CSL script: for one picture line, the GA's view of where the display starts: per tick
//! around `from..to` (picture pixels), the colour, DISPEN, the HSYNC as the GA sees it and the video
//! mode in use. `probe_display_start <script.csl> <picture line> <from_x> <to_x>`
use amsrust::chip::Chip;
use amsrust::csl_runner::CslRunner;
use amsrust::ga::GateArrayPin;
use amsrust::monitor::Monitor;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let line: isize = a.next().expect("picture line").parse().unwrap();
    let from: isize = a.next().expect("from").parse().unwrap();
    let to: isize = a.next().expect("to").parse().unwrap();
    let mut runner = CslRunner::new("/tmp/probe_display_start_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    println!("gate array: {:?}", mb.ga().gate_array_type());
    let mut monitor = Monitor::default();
    let mut printed_palette = false;
    mb.crtc_mut().start_write_log();
    let mut writes_shown = false;
    let mut frames = 0;
    while frames < 2 {
        let (rgb, sync) = mb.tick();
        monitor.receive(rgb, sync);
        if frames == 1 {
            let (x, beam_line) = monitor.beam();
            let (crop_x, crop_y) = monitor.crop_origin();
            let y = beam_line as isize - crop_y as isize;
            let px = x as isize - crop_x;
            if y == line && !printed_palette && px >= from {
                printed_palette = true;
                let pal: Vec<String> = mb
                    .ga()
                    .palette()
                    .iter()
                    .enumerate()
                    .map(|(i, ink)| {
                        let c: amsrust::ga::RgbLevels = (*ink).into();
                        format!(
                            "{i}:({:.0},{:.0},{:.0})",
                            c.r() * 2.55,
                            c.g() * 2.55,
                            c.b() * 2.55
                        )
                    })
                    .collect();
                println!("palette: {}", pal.join(" "));
            }
            let writes = mb.crtc_mut().take_write_log();
            if y == line || y + 1 == line {
                for (c0, c4, c9, r, v) in writes {
                    println!(
                        "CRTC write at picture line {y} x={px}: R{r}={v} (C0={c0} C4={c4} C9={c9})"
                    );
                }
                if !writes_shown && y == line {
                    writes_shown = true;
                    println!(
                        "R0..R3 at line start: {:?}",
                        (0..4).map(|r| mb.crtc().regs_r(r)).collect::<Vec<_>>()
                    );
                }
            }
            if y == line && (from..to).contains(&px) {
                println!(
                    "x={px:4} rgb=({:3.0},{:3.0},{:3.0}) DISPEN={} hsync_ga={} mode={:?} next={:?} rmr={:02x} crtc_hsync={} C0={} crtc_DE={}",
                    rgb.r(),
                    rgb.g(),
                    rgb.b(),
                    mb.ga().get_pin(GateArrayPin::DISPEN) as u8,
                    mb.ga().hsync_ga() as u8,
                    mb.ga().active_video_mode(),
                    mb.ga().next_video_mode(),
                    mb.ga().rmr(),
                    mb.crtc().hsync() as u8,
                    mb.crtc().hcc,
                    mb.crtc().get_pin(amsrust::crtc::CrtcPin::DE) as u8
                );
            }
        }
        if monitor.take_frame().is_some() {
            frames += 1;
        }
    }
}
