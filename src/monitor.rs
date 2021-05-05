/// https://www.cpcwiki.eu/forum/amstrad-cpc-hardware/cpc-max-screen-resolution/
/// https://en.wikipedia.org/wiki/PAL
/// http://martin.hinner.info/vga/pal.html

use derive_more::{Add, AddAssign, From, Into, Sub, SubAssign};
use image::*;

/// This is the total duration to trat a line
const RASTER_LINE_LENGTH: usize = 64 * 8 * 2; // (1024) In fact it is far smaller
/// This is the amount of visible pixels before retracing
const MONITOR_ACTIVE_PIXELS: usize = 52*8*2; // (832)
const MONITOR_HEIGHT: usize = 312; //.5

const MINIUM_HSYNC_SIZE: u32 = 8*8*2;

const MONITOR_VISIBLE_LINES: usize = 288;
const MONITOR_VISIBLE_PIXELS: usize = 768;

const MONITOR_HORIZONTAL_START_RETRACE: usize = MONITOR_ACTIVE_PIXELS;
const MONITOR_HORIZONTAL_RETRACE_DURATION: usize = (RASTER_LINE_LENGTH-MONITOR_HORIZONTAL_START_RETRACE) * 80 / 100;
const MONITOR_VERTICAL_START_RETRACE: usize = 312;

const MONITOR_HOLD: usize = RASTER_LINE_LENGTH - MONITOR_HORIZONTAL_START_RETRACE - MONITOR_HORIZONTAL_RETRACE_DURATION;

const HSYNC_DEBUG_COLOR: Rgb<u8> = Rgb([0, 128, 255]);
const VSYNC_DEBUG_COLOR: Rgb<u8> = Rgb([0, 64, 255]);

const HSYNC_COLOR: Rgb<u8> = Rgb([0, 0, 0]);
const VSYNC_COLOR: Rgb<u8> = Rgb([0, 0, 0]);

/// Drawing texture
pub struct Texture(RgbImage);
impl Default for Texture {
    fn default() -> Self {
        Self(RgbImage::new(RASTER_LINE_LENGTH as u32, MONITOR_HEIGHT as u32 *2))
    }
}

#[derive(Clone, Copy, Debug, Default, Add, AddAssign, From, PartialEq, Into)]
struct VerticalPosition(usize);

#[derive(Clone, Copy, Debug, Default, Add, AddAssign, From, PartialEq, Into)]
struct HorizontalPosition(usize);

/// Electron gun
#[derive(Default)]
struct Gun {
    x: HorizontalPosition,
    y: VerticalPosition,
}

impl Gun {
    fn at(y: VerticalPosition) -> Self {
        Self { x: 0.into(), y }
    }
}



#[derive(Clone, Copy, Debug, Sub, SubAssign, From, Into, PartialEq)]
struct VHold(usize);
impl Default for VHold {
    fn default() -> Self {
        VHold(16 * (MONITOR_HORIZONTAL_START_RETRACE + MONITOR_HORIZONTAL_START_RETRACE * 3 / 10))
    }
}

#[derive(Clone, Copy, Debug, Sub, SubAssign, From, Into, PartialEq)]
struct HHold(usize);
impl Default for HHold {
    fn default() -> Self {
        HHold(MONITOR_HOLD)
    }
}

/// Counter for Vertical Retrace
#[derive(Clone, Copy, Debug, Sub, SubAssign, From, PartialEq, PartialOrd)]
struct VRetrace(usize);
impl Default for VRetrace {
    fn default() -> Self {
       // Self(MONITOR_VERTICAL_START_RETRACE * 3 / 10 -1)
       Self(0)
    }
}

/// Counter for Horizontal Retrace
#[derive(Clone, Copy, Debug, Sub, SubAssign, From, PartialEq, PartialOrd)]
struct HRetrace(usize);
impl Default for HRetrace {
    fn default() -> Self {
        Self(MONITOR_HORIZONTAL_RETRACE_DURATION)
    }
}

#[derive(Clone, Copy, Debug)]
struct Brightness(u8);

impl Default for Brightness {
    fn default() -> Self {
        Brightness(128)
    }
}

impl Texture {
    /// Set the pixel in the texture. Remember each pixels is drawn on two lines to properly simulates mode 2
    pub fn set(&mut self, y: usize, x: usize, pixel: Rgb<u8>) {

        let x = x as u32;
        let y = y as u32;

        self.0.put_pixel(x, y*2+0, pixel);
        self.0.put_pixel(x, y*2+1, pixel);
    }

    /// Save the texture in file p
    pub fn save<P:AsRef<std::path::Path>>(&self, p: P) -> Result<(), ImageError> {
        self.0.save(p.as_ref())
    }
}

/// The monitor can be waiting the vsync signal for a horizontal synchronisation
/// or displaying the bytes.
/// I am not 100% sure if it is necessary to model these states as they must accor faster than the GA reaction
enum MonitorState {
    /// Counter for vsync
    WaitVerticalSynchronization(VHold),
    /// Counter for hsync wait (next line, duration)
    WaitHorizontalSynchronization(VerticalPosition, HHold),
    /// Move to the beginning of next line with showing anything (next line)
    HorizontalRetrace(VerticalPosition, HRetrace),
    /// Move back to the beginning of the screen
    VerticalRetrace(VRetrace),
    /// State where the electron gun is moving
    Display(Gun),
}

impl Default for MonitorState {
    fn default() -> Self {
        MonitorState::WaitVerticalSynchronization(VHold::default())
    }
}

impl MonitorState {
    pub fn is_displaying(&self) -> bool {
        match self {
            MonitorState::Display(_) => true,
            _ => false,
        }
    }

    pub fn is_in_vertical_retrace(&self) -> bool {
        match self {
            MonitorState::VerticalRetrace(_) => true,
            _ => false,
        }
    }
    pub fn vertical_retrace_step_mut(&mut self) -> Option<&mut VRetrace> {
        match self {
            MonitorState::VerticalRetrace(ref mut count) => Some(count),
            _ => None,
        }
    }

    pub fn is_in_horizontal_retrace(&self) -> bool {
        match self {
            MonitorState::HorizontalRetrace(_, _) => true,
            _ => false,
        }
    }
    pub fn horizontal_retrace_step_mut(&mut self) -> Option<&mut HRetrace> {
        match self {
            MonitorState::HorizontalRetrace(_, ref mut step) => Some(step),
            _ => None,
        }
    }

    pub fn is_waiting_vsync(&self) -> bool {
        match self {
            MonitorState::WaitVerticalSynchronization(_) => true,
            _ => false,
        }
    }

    pub fn wait_vsync_counter_mut(&mut self) -> Option<&mut VHold> {
        match self {
            MonitorState::WaitVerticalSynchronization(h) => Some(h),
            _ => None,
        }
    }

    pub fn is_waiting_hsync(&self) -> bool {
        match self {
            MonitorState::WaitHorizontalSynchronization(_, _) => true,
            _ => false,
        }
    }

    pub fn wait_hsync_counter_mut(&mut self) -> Option<&mut HHold> {
        match self {
            MonitorState::WaitHorizontalSynchronization(_, h) => Some(h),
            _ => None,
        }
    }

    pub fn gun_mut(&mut self) -> Option<&mut Gun> {
        match self {
            Self::Display(ref mut g) => Some(g),
            _ => None,
        }
    }

    pub fn get_vertical_position(&self) -> Option<VerticalPosition> {
        match self {
            Self::Display(g) => Some(g.y),
            Self::HorizontalRetrace(ref y, _) => Some(*y),
            Self::WaitHorizontalSynchronization(ref y, _) => Some(*y),
            _ => None,
        }
    }
}

struct Monitor {
    /// Monitor state
    state: MonitorState,
    /// Maximal Vertical Hold value (button on real monitor)
    vhold: VHold,
    /// Maximum Horizontal Hold value (button on real monitor)
    hhold: HHold,
    /// Brightness configuration
    brightness: Brightness,
    /// Amount of time waiting the vsync signal
    wait_vsync: usize,

    /// full texture of the screen (need to be cropped to remove the unwanted stuff)
    full_texture: Texture,

    vsync_color: Rgb<u8>,
    hsync_color: Rgb<u8>,

    previous_hsync: bool,
}


impl Default for Monitor {
    fn default() -> Self {
        Monitor {
            state: Default::default(),
            vhold: Default::default(),
            hhold: Default::default(),
            brightness: Default::default(),
            wait_vsync: Default::default(),
            full_texture: Default::default(),

            vsync_color: VSYNC_COLOR,
            hsync_color: HSYNC_COLOR,

            previous_hsync: false
        }
    }
}

impl Monitor {

    fn debug() -> Self {
        Monitor {
            state: Default::default(),
            vhold: Default::default(),
            hhold: Default::default(),
            brightness: Default::default(),
            wait_vsync: Default::default(),
            full_texture: Default::default(),

            vsync_color: VSYNC_DEBUG_COLOR,
            hsync_color: HSYNC_DEBUG_COLOR,

            previous_hsync: false
        }
    }
    /// Handle the information send from the CPC
    pub fn receive(&mut self, r: f32, g: f32, b: f32, vsync: bool, hsync: bool) {
        if self.state.is_in_vertical_retrace() {
            // Gun is moving from bottom right to top left
            let step = self.state.vertical_retrace_step_mut().unwrap();
            if *step <= 1.into() {
                self.state = MonitorState::WaitVerticalSynchronization(self.vhold);
            }
            else {
                *step -= 1.into();
            }
        }

        if self.state.is_in_horizontal_retrace() {
            // Gun is moving for right to left / next line
            let step = self.state.horizontal_retrace_step_mut().unwrap();
            if *step <= 1.into() {
                self.state = MonitorState::WaitHorizontalSynchronization(
                    self.state.get_vertical_position().unwrap(),
                    self.hhold,
                );
            }
            else {
                *step -= 1.into();
            }
        }

        if self.state.is_waiting_vsync() {
            // Gun waits for vertical synchronization
            let wait = self.state.wait_vsync_counter_mut().unwrap();
            *wait -= 1.into();

            if *wait == 0.into() || vsync {
                self.state = MonitorState::Display(Gun::default());
            }
        }

        if self.state.is_waiting_hsync() {
            // Gun waits for horizontal synchronization
            let wait = self.state.wait_hsync_counter_mut().unwrap();

            if !hsync {
                *wait -= 1.into();
            }

            if (*wait == 0.into() && !hsync) || (!hsync && self.previous_hsync) {
                self.state =
                    MonitorState::Display(Gun::at(self.state.get_vertical_position().unwrap()));
            }
        }

        if self.state.is_displaying() {
            // really make the pixel display
            let rgb = self.compute_rgb(r, g, b);
            let gun = self.state.gun_mut().unwrap();

            if vsync {
                self.full_texture.set(gun.y.into(), gun.x.into(), self.vsync_color);
            }
            else if hsync {
                self.full_texture.set(gun.y.into(), gun.x.into(), self.hsync_color);
            }
            else {
                self.full_texture.set(gun.y.into(), gun.x.into(), rgb);
            }

            // move the gun to the next step
            gun.x += 1.into();
            // TODO - rewrite that, it is probably wrong
            if gun.x == MONITOR_HORIZONTAL_START_RETRACE.into() {
                gun.x = 0.into();
                gun.y += 1.into();

                if gun.y == MONITOR_VERTICAL_START_RETRACE.into() {
                    self.state = MonitorState::VerticalRetrace(VRetrace::default());
                } else {
                    self.state = MonitorState::HorizontalRetrace(gun.y, HRetrace::default());
                }
            }
        }

        self.previous_hsync = vsync;
    }

    /// Compute the RGB color from the received intensities
    fn compute_rgb(&self, r: f32, g: f32, b: f32) -> Rgb<u8> {
        /// TODO play with luminance to update the brightness
        let to_u8 = |v: f32| -> u8 { (v * 255f32).min(255f32) as u8 };

        Rgb([to_u8(r), to_u8(g), to_u8(b)])
    }
}


#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_wait_vsync_delay() {
        let mut mon = Monitor::default();

        for i in (0.. usize::from(VHold::default())) {
            assert!(mon.state.is_waiting_vsync());
            mon.receive(0.5, 0.5, 0.5, false, false);
        }

        assert!(!mon.state.is_waiting_vsync());
    }


    #[test]
    fn test_wait_vsync_no_delay() {
        let mut mon = Monitor::default();

        for i in (0..2) {
            assert!(mon.state.is_waiting_vsync());
            mon.receive(0.5, 0.5, 0.5, false, false);
        }

        assert!(mon.state.is_waiting_vsync());
        mon.receive(0.5, 0.5, 0.5, true, false);
        assert!(!mon.state.is_waiting_vsync());
    }

    #[test]
    fn test_color() {
        let mut mon = Monitor::default();
        assert_eq!(
            mon.compute_rgb(0., 0., 0.),
            Rgb([0, 0, 0])
        );

        assert_eq!(
            mon.compute_rgb(1., 1., 1.),
            Rgb([255, 255, 255])
        );
    }

    #[test]
    fn test_standard_screen() {
        let mut monitor = Monitor::debug();

        let r7 = 0;
        let r4 = 38;
        let r9 = 7;
        let r1 = 64;
        let r2 = 46;

        for char_line in 0..=r4 {
            for scan_line in 0..=r9 {
                let vsync = (char_line*8 + scan_line) < 16; 

                for char in 0..=r1 {
                    let hsync = char >= r2 && char < 60;
                    for pixel in 0..16 { // ga cycles

                        if hsync || vsync {
                            monitor.receive(0., 0., 0., vsync, hsync );
                        }
                        else {
                            monitor.receive(
                                (char_line+1) as f32 / r4 as f32, 
                                (char+1) as f32 / r1 as f32,
                                (scan_line+1) as f32 / r9 as f32, 
                                vsync, hsync );
                        }
                    }
                }
            }
        }

        monitor.full_texture.save("test_monitor_standard_screen.png").unwrap();
    }
}