use derive_more::{Add, AddAssign, From, Into, Sub, SubAssign};

/// Drawing texture
pub struct Texture([u8; MONITOR_HEIGHT * MONITOR_WIDTH * 3 * 2]);
impl Default for Texture {
    fn default() -> Self {
        Self([0; MONITOR_HEIGHT * MONITOR_WIDTH * 3 * 2])
    }
}
pub struct Rgb(u8, u8, u8);

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

const MONITOR_WIDTH: usize = 64 * 8 * 2; // In fact it is far smaller
const MONITOR_HEIGHT: usize = 312 * 2;

/// No idea if it is good or no
const MONITOR_HORIZONTAL_START_RETRACE: usize = 96 * 8;
const MONITOR_VERTICAL_START_RETRACE: usize = 312;

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
        HHold(4)
    }
}

/// Counter for Vertical Retrace
#[derive(Clone, Copy, Debug, Sub, SubAssign, From, PartialEq)]
struct VRetrace(usize);
impl Default for VRetrace {
    fn default() -> Self {
        Self(MONITOR_VERTICAL_START_RETRACE * 3 / 10)
    }
}

/// Counter for Horizontal Retrace
#[derive(Clone, Copy, Debug, Sub, SubAssign, From, PartialEq)]
struct HRetrace(usize);
impl Default for HRetrace {
    fn default() -> Self {
        Self(MONITOR_HORIZONTAL_START_RETRACE * 3 / 10)
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
    pub fn set(&mut self, y: usize, x: usize, rgb: Rgb) {
        self.0[(y * 2 + 0) * (MONITOR_WIDTH * 3) + 3 * x + 0] = rgb.0;
        self.0[(y * 2 + 1) * (MONITOR_WIDTH * 3) + 3 * x + 0] = rgb.0;

        self.0[(y * 2 + 0) * (MONITOR_WIDTH * 3) + 3 * x + 1] = rgb.1;
        self.0[(y * 2 + 1) * (MONITOR_WIDTH * 3) + 3 * x + 1] = rgb.1;

        self.0[(y * 2 + 0) * (MONITOR_WIDTH * 3) + 3 * x + 2] = rgb.2;
        self.0[(y * 2 + 1) * (MONITOR_WIDTH * 3) + 3 * x + 2] = rgb.2;
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

#[derive(Default)]
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
}

impl Monitor {
    /// Handle the information send from the CPC
    pub fn receive(&mut self, r: f32, g: f32, b: f32, vsync: bool, hsync: bool) {
        if self.state.is_in_vertical_retrace() {
            // Gun is moving from bottom right to top left
            let step = self.state.vertical_retrace_step_mut().unwrap();
            *step -= 1.into();
            if *step == 0.into() {
                self.state = MonitorState::WaitVerticalSynchronization(self.vhold);
            }
        }

        if self.state.is_in_horizontal_retrace() {
            // Gun is moving for right to left / next line
            let step = self.state.horizontal_retrace_step_mut().unwrap();
            *step -= 1.into();
            if *step == 0.into() {
                self.state = MonitorState::WaitHorizontalSynchronization(
                    self.state.get_vertical_position().unwrap(),
                    self.hhold,
                );
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
            *wait -= 1.into();

            if *wait == 0.into() || hsync {
                self.state =
                    MonitorState::Display(Gun::at(self.state.get_vertical_position().unwrap()));
            }
        }

        if self.state.is_displaying() {
            // really make the pixel display
            let rgb = self.compute_rgb(r, g, b);
            let gun = self.state.gun_mut().unwrap();
            self.full_texture.set(gun.y.into(), gun.x.into(), rgb);

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
    }

    /// Compute the RGB color from the received intensities
    fn compute_rgb(&self, r: f32, g: f32, b: f32) -> Rgb {
        /// TODO play with luminance to update the brightness
        let to_u8 = |v: f32| -> u8 { (v * 255f32).max(255f32) as u8 };

        Rgb(to_u8(r), to_u8(g), to_u8(b))
    }
}
