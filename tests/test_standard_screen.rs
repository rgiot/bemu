/// This test aims at chcking the behavior of GA/monitor for a standard screen
use amsrust::crtc::*;
use amsrust::ga::*;
use amsrust::monitor::*;
use amsrust::chip::*;

fn geneate_screen(img: &[u8;0x4000], fname: &str) {

    let mut crtc = Crtc::default();
    let mut ga = GateArray::default();
    let mut monitor = Monitor::debug();


    for char_line in 0..=crtc.do_read_register(4) {
        for scan_line in 0..=crtc.do_read_register(9) {
            for char in 0..=crtc.do_read_register(1) {
                for pixel in 0..16 { // ga cycles
                    ga.tick();
                    if crtc.is_activated(){
                        crtc.tick();
                    }

                    let rgb = ga.rgb();
                    let vsync = ga.get_pin(GateArrayPin::VSYNC);
                    let hsync = ga.get_pin(GateArrayPin::HSYNC);

                    monitor.receive(
                        rgb.0,
                        rgb.1,
                        rgb.2,
                        vsync,
                        hsync
                    );
                }
            }
        }
    }


    monitor.image().save("test_standard_screen_pic.png").unwrap();

}