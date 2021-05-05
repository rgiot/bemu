/// This test aims at chcking the behavior of GA/monitor for a standard screen
use amsrust::crtc::Crtc;
use amsrust::ga::GateArray;
use amsrust::monitor::Monitor;

fn geneate_screen(img: &[u8;0x4000], img: &str) {

    let mut crtc = Crtc::default();
    let mut ga = GateArray::default();
    let mut monitor = Monitor::default();

    loop {
        
    }
    while (!monitor.frame_finished());
}