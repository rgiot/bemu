/// This test aims at chcking the behavior of GA/monitor for a standard screen
use amsrust::crtc::*;
use amsrust::ga::*;
use amsrust::monitor::*;
use amsrust::chip::*;
use std::collections::HashSet;

fn beginning_steps(ga: &mut GateArray, crtc: &mut Crtc) {
    ga.save_pins_state();
    crtc.save_pins_state();
    sync_pins(
        ga,
        &[GateArrayPin::CCLK],
        crtc,
        &[CrtcPin::CCLK],
    );
    sync_pins(
        crtc,
        &[CrtcPin::VS, CrtcPin::HS, CrtcPin::DE],
        ga,
        &[GateArrayPin::VSYNC, GateArrayPin::HSYNC, GateArrayPin::DISPEN],
    );

    assert_eq!(
        ga.get_pin(GateArrayPin::CCLK),
        crtc.get_pin(CrtcPin::CCLK),
    );
}

fn end_steps(ga: &mut GateArray, crtc: &mut Crtc, monitor: &mut Monitor) {
    let is_border = !ga.get_pin(GateArrayPin::DISPEN);
    // ga cycles
    ga.tick();
    if crtc.is_activated(){
        crtc.tick();

    }

    let rgb = dbg!(ga.rgb());
    let palette = dbg!(ga.palette()); 
    dbg!(palette[0].to_rgb_levels());


    let vsync = ga.get_pin(GateArrayPin::VSYNC);
    let hsync = ga.get_pin(GateArrayPin::HSYNC);

    if !vsync && !hsync {
        if is_border {
            assert_eq!(
                rgb,
                palette[16].to_rgb_levels()
            )
        }
        else {
            // check mode 1 colors
        assert!(
            rgb == palette[0].to_rgb_levels() 
            || rgb == palette[1].to_rgb_levels() 
            || rgb == palette[2].to_rgb_levels() 
            || rgb == palette[3].to_rgb_levels()
        );
        }
        
    } else {
        assert_eq!(
            RgbLevels::from((0., 0., 0.)),
            rgb
        );
    }


   // let rgb : RgbLevels= palette[1].to_rgb_levels();
    monitor.receive(
        rgb,
        vsync,
        hsync
    );

    ga.manage_clocks();
}

fn generate_screen(bank: &[u8;0x4000], fname: &str, palette: [Ink;17], mode: VideoMode){

    // Configure the crtc just before raising the vsync
    let mut crtc = Crtc::default();
    crtc.do_write_register(12, 0x30);
    crtc.do_write_register(13, 0x00);
    crtc.vlc = 7;
    crtc.vcc = 29;
    crtc.hcc = 63;
    crtc.base = 0x3000;
    crtc.set_address_on_pins();

    // Configure the monitor in a wait state (not a big deal, it happens one char too soon)
    let mut monitor = Monitor::wait_vsync();


    let mut ga = GateArray::default();
    ga.set_palette(palette);
    ga.force_active_video_mode(mode);
    ga.set_video_mode(mode);
    ga.set_pin(GateArrayPin::CCLK, false);

    /*
    // let one second  pass to properly synchronize screen
    for f in 0..50 {
        for char_line in 0..=crtc.get_r4() {
            for scan_line in 0..=crtc.get_r9() {
                for char in 0..=crtc.get_r0() {
                    for pixel in 0..16 { 
                        beginning_steps(&mut ga, &mut crtc);

                        if  ga.need_memory_byte(){
                            ga.set_memory_byte(0);
                        }
                        
                        end_steps(&mut ga, &mut crtc, &mut monitor);

                    }
                }
            }
        }
    }
    */

    // and make the real tests

    assert_eq!(ga.get_pin(GateArrayPin::CCLK), false);
    let crtc_address = crtc.get_address_from_pins();
    println!("0x{:x}", crtc_address);

    for char_line in 0..=crtc.get_r4() {
        for scan_line in 0..=crtc.get_r9() {
            for char in 0..=crtc.get_r0() {
                for pixel in 0..16 { 
                    beginning_steps(&mut ga, &mut crtc);

                    // TODO reimplement this stuff in a more
                    // realistic way
                    if  dbg!(ga.need_memory_byte()){
                        // Retreive CRTC address
                        let crtc_address = dbg!(crtc.get_address_from_pins());
                        dbg!(crtc_address);
                        assert!(crtc_address>=0xc000);

       
                        // Feed GA with byte
                        let byte = bank[crtc_address as usize-0xc000];
                        ga.set_memory_byte(byte); // TODO do it with its pins
                    }
                    
                    end_steps(&mut ga, &mut crtc, &mut monitor);
                }
            }
        }
    }


    monitor.image().save(fname).unwrap();

}

#[test]
fn test_crtc_catart(){
    let bank = include_bytes!("screen.scr");
    let fname = "test_standard_screen_pic.png";
    let palette: [Ink;17]= [
        0.into(),
        8.into(),
        13.into(),
        18.into(),
        0.into(),
        0.into(),
        0.into(),
        0.into(),
        0.into(),
        0.into(),
        0.into(),
        0.into(),
        0.into(),
        0.into(),
        0.into(),
        0.into(),
        0.into(),
    ];
    let mode = VideoMode::Mode1;
    generate_screen(&bank, fname, palette, mode);
}