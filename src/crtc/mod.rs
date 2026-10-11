mod asic;
mod model;
mod type0;
mod type1;
mod type2;

pub use model::{CrtcModel, CrtcType, DisplaySkew, ReadContext, Registers, NB_REGISTERS};

use crate::bits::*;
use crate::chip::*;

use num_enum::IntoPrimitive;
use num_enum::TryFromPrimitive;

#[allow(non_camel_case_types)]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u64)]
pub enum CrtcPin {
    D0 = 1 << 0,
    D1 = 1 << 1,
    D2 = 1 << 2,
    D3 = 1 << 3,
    D4 = 1 << 4,
    D5 = 1 << 5,
    D6 = 1 << 6,
    D7 = 1 << 7,

    /// Register select (A8)
    RS = 1 << 8,
    /// Chip Select (14)
    CS = 1 << 9,
    /// Read Write (A9)
    RW = 1 << 10,
    /// Enable the CRTC
    EN = 1 << 11,

    /// Vertical Sync
    VS = 1 << 12,
    /// Horizontal Sync
    HS = 1 << 13,

    /// Display Enable
    DE = 1 << 14,

    /// Clock
    CCLK = 1 << 15,

    /// Reset
    RESET = 1 << 16,

    MA13 = 1 << 17,
    MA12 = 1 << 18,
    MA11 = 1 << 19,
    MA10 = 1 << 20,
    MA9 = 1 << 21,
    MA8 = 1 << 22,
    MA7 = 1 << 23,
    MA6 = 1 << 24,
    MA5 = 1 << 25,
    MA4 = 1 << 26,
    MA3 = 1 << 27,
    MA2 = 1 << 28,
    MA1 = 1 << 29,
    MA0 = 1 << 30,

    RA0 = 1 << 31,
    RA1 = 1 << 32,
    RA2 = 1 << 33,
    RA3 = 1 << 34,
    RA4 = 1 << 35,
}

impl Into<u64> for CrtcPin {
    fn into(self) -> u64 {
        self as u64
    }
}

const CRTC_PIN_DATA_BUS: [CrtcPin; 8] = [
    CrtcPin::D7,
    CrtcPin::D6,
    CrtcPin::D5,
    CrtcPin::D4,
    CrtcPin::D3,
    CrtcPin::D2,
    CrtcPin::D1,
    CrtcPin::D0,
];

const CRTC_PIN_ADDRESS_BUS: [CrtcPin; 16] = [
    CrtcPin::MA13,
    CrtcPin::MA12,
    CrtcPin::RA2,
    CrtcPin::RA1,
    CrtcPin::RA0,
    CrtcPin::MA9,
    CrtcPin::MA8,
    CrtcPin::MA7,
    CrtcPin::MA6,
    CrtcPin::MA5,
    CrtcPin::MA4,
    CrtcPin::MA3,
    CrtcPin::MA2,
    CrtcPin::MA1,
    CrtcPin::MA0,
    CrtcPin::CCLK,
];

#[derive(Debug)]
/// https://www.grimware.org/doku.php/documentations/devices/crtc
/// https://neuro-sys.github.io/2019/10/01/amstrad-cpc-crtc.html
pub struct Crtc {
    pins: PinsBuffer,

    /// Address Register / Selection of the working register
    ar: u8,
    /// Status register (crtvc 1)
    sr: u8,
    /// R0-R17, masked to the bits the chip stores (see [`CrtcModel::write_mask`]):
    /// R0 horizontal total, R1 horizontal displayed, R2 HSYNC position,
    /// R3 sync widths, R4 vertical total, R5 vertical total adjust,
    /// R6 vertical displayed, R7 VSYNC position, R8 interlace and skew,
    /// R9 maximum scan line, R10/R11 cursor, R12/R13 display start,
    /// R14/R15 cursor address, R16/R17 light pen.
    regs: Registers,

    ///  	The Horizontal Character Counter (an internal CRTC register). Keeps track of the current horizontal character number. When this value equals R1 of the CRTC (horizontal displayed) display output is disabled for the rest of the current line. When this value equals R0 of the CRTC (horizontal total), display output is enabled and this value is reset to zero. When this value equals R2 of the CRTC (horizontal sync position) a new scan line may be started depending on the current HSYNC position of the monitor.
    pub hcc: u8,
    /// The Vertical Character Counter (an internal CRTC register). Keeps track of the current vertical character number. When this value equals R6 of the CRTC (vertical displayed), display output is disabled. When this value equals R4 of the CRTC (vertical total), display output is enabled and the display base address is reset. When this value equals R7 of the CRTC (vertical sync position) a Vertical Sync occurs, the display will start drawing from the top if the current VDUR value is within a threshold range.
    pub vcc: Bit8Max7,
    ///  	The Vertical Line Counter (an internal CRTC register). Keeps track of the current raster line within the character. When this equals R9 (maximum scan line) of the CRTC the VCC will be incremented next line.
    pub vlc: Bit8Max5,
    ///  	The Vertical Sync counter (an internal emulator register). When a Vertical Sync occurs this register is set to zero. When a frame flyback is active this will be displayed in light blue and count the lines in the frame flyback period.
    pub vsc: u8,
    /// The Horizontal Display Counter shows the current horizontal position on the monitor being drawn.
    pub hdc: u8,
    /// The Vertical Total Adjust Counter is shown in light blue when the end of the current CRTC frame is reached (after VCC = R4) and CRTC Register 5 has a value other than zero. This counts up to the value in CRTC register 5 before the next frame is started.
    pub vtac: u8,
    /// True while the CRTC is between "VCC just reached R4" and "VTAC just reached R5":
    /// VCC/VLC are held and only VTAC advances, one extra raster line at a
    /// time, until the frame's total scan-line count has been topped up to
    /// exactly R5 lines past the nominal (R4+1)*(R9+1) - the mechanism a
    /// real 6845 uses to hit an exact line count (e.g. 312 for PAL) when
    /// (R4+1)*(R9+1) alone doesn't divide evenly.
    in_vertical_adjust: bool,
    /// The "last line of the frame" state of CRTC 0 (see
    /// [`CrtcModel::has_last_line_state`]).
    last_line: bool,
    /// CRTC 0: the additional line management armed by default on a last line
    /// found at C0<2, disarmed when C0 reaches 2 (so never on a 2 character
    /// line), and the R5 > 0 request seen before C0=3 (ACCC 13.2.1).
    additional_armed: bool,
    additional_r5: bool,
    /// Diagnostics: every register write as (C0, C4, C9, register, value).
    write_log: Option<Vec<(u8, u8, u8, u8, u8)>>,
    /// The Horizontal Sync Counter counts the number of cycles during a HSYNC. This is shown in light blue when the CRTC Horizontal Sync period is active.
    pub hsc: u8,

    /// CRTC-VMA (ACCC §17.1/§20.2): the live 14-bit video memory address
    /// counter. Genuinely incremented once per character, whatever the
    /// display state - a real 6845 has no idea it's drawing a border, it
    /// just keeps counting (ACCC §17.3: "the VMA pointer continues to
    /// count", even throughout the BORDER). This is what actually goes out
    /// on the MA/RA pins (see [`Crtc::set_address_on_pins`]), not a formula
    /// recomputed from `vcc`/`hcc` - which matters the moment a program
    /// changes R1 or R12/R13 mid-frame (split-screen/dynamic-R1 techniques,
    /// ACCC §17.3, §20.3): a formula would retroactively apply the new
    /// value to rows it was never actually in effect for.
    crtc_vma: u16,
    /// CRTC-VMA' (ACCC §17.1): where the *next* row starts. Latched from
    /// `crtc_vma` when C0 reaches R1 on that row's last scan line (C9=R9) -
    /// so a row whose R1 was never reached that line (R1>R0) never gets its
    /// next-row address updated, and repeats verbatim, exactly like real
    /// hardware (ACCC §17.2.2/§17.4). Transferred into `crtc_vma` at the
    /// start of every following line (C0=0); the very first line of a
    /// frame is special-cased per CRTC type (ACCC §20.3, §17.4.1/§17.4.2),
    /// see [`Crtc::reload_vma_if_needed`].
    crtc_vma_prime: u16,

    model: &'static dyn CrtcModel,

    /// Whether the last completed line was in the vertical border by R6
    /// (the UM6845R status register's V bit).
    vertical_border: bool,
    /// The horizontal BORDER condition of the last characters, newest first:
    /// the R8 display skew delays it by 1 or 2 characters (ACCC §19.2.3).
    border_history: [bool; 3],
    /// C4 reached R6: BORDER until the next frame (CRTC 0 and 2, ACCC §18.2.2).
    r6_border: bool,
    /// CRTC 0 with R0=0 (ACCC §13.2.4): how many times C0 has wrapped to 0 since
    /// R0 became 0 (0, 1, 2 and more); C9 is frozen from the second one on.
    r0_zero_wraps: u8,
    /// C9 had reached R9 on the first of them: C4 moves on the second.
    r0_zero_c4_pending: bool,
    /// R6 was 0 during this frame's first line: the C4=R6 conflict of ACCC §18.3.2.
    r6_zero_seen: bool,
    /// State of the noise source behind the floating values some chips
    /// return (xorshift32: reproducible, unlike an OS random number).
    noise: u32,
}

impl Default for Crtc {
    fn default() -> Self {
        let mut crtc = Crtc {
            pins: Default::default(),
            ar: Default::default(),
            sr: Default::default(),
            regs: Default::default(),
            hcc: Default::default(),
            vcc: Default::default(),
            vlc: Default::default(),
            vsc: 0, // meaningless when no vsync is present
            hsc: 0, // meaningless when no hsync is present
            hdc: 0,
            vtac: 0,
            in_vertical_adjust: false,
            last_line: false,
            additional_armed: false,
            additional_r5: false,
            write_log: None,
            crtc_vma: 0,
            crtc_vma_prime: 0,
            model: CrtcType::default().model(),
            vertical_border: false,
            border_history: [false; 3],
            r6_border: false,
            r0_zero_wraps: 0,
            r0_zero_c4_pending: false,
            r6_zero_seen: false,
            noise: 0x2545_f491,
        };
        crtc.reset();
        crtc
    }
}
impl From<&CrtcType> for Crtc {
    fn from(from: &CrtcType) -> Self {
        Crtc::from(*from)
    }
}

impl From<CrtcType> for Crtc {
    fn from(from: CrtcType) -> Self {
        let mut crtc = Crtc {
            model: from.model(),
            ..Default::default()
        };
        crtc.reset();
        crtc
    }
}

impl Crtc {
    pub fn r0(&self) -> u8 {
        self.regs[0]
    }
    pub fn r1(&self) -> u8 {
        self.regs[1]
    }
    pub fn r2(&self) -> u8 {
        self.regs[2]
    }
    pub fn r3(&self) -> u8 {
        self.regs[3]
    }
    pub fn r4(&self) -> u8 {
        self.regs[4]
    }
    pub fn r5(&self) -> u8 {
        self.regs[5]
    }
    pub fn r6(&self) -> u8 {
        self.regs[6]
    }
    pub fn r7(&self) -> u8 {
        self.regs[7]
    }
    pub fn r8(&self) -> u8 {
        self.regs[8]
    }
    pub fn r9(&self) -> u8 {
        self.regs[9]
    }
    pub fn r10(&self) -> u8 {
        self.regs[10]
    }
    pub fn r11(&self) -> u8 {
        self.regs[11]
    }
    pub fn r12(&self) -> u8 {
        self.regs[12]
    }
    pub fn r13(&self) -> u8 {
        self.regs[13]
    }
    pub fn r14(&self) -> u8 {
        self.regs[14]
    }
    pub fn r15(&self) -> u8 {
        self.regs[15]
    }
    pub fn r16(&self) -> u8 {
        self.regs[16]
    }
    pub fn r17(&self) -> u8 {
        self.regs[17]
    }
    pub fn ar(&self) -> u8 {
        self.ar
    }
}

impl Crtc {

    /// Execute the CRTC register selection
    pub fn select_crtc_register(&mut self) {
        let reg = self.get_data_from_pins();
        self.ar = reg & 0b11111;
    }

    /// Execute the CRTC register write
    pub fn write_crtc_register(&mut self) {
        let val = self.get_data_from_pins();
        if self.is_register_writable(self.ar) {
            self.do_write_register(self.ar, val);
        }
    }

    /// Execute the CRTC register status (read of &BE00): what the data bus
    /// carries is the chip's business.
    pub fn status_register(&mut self) {
        let data = self.model.read_status(&self.read_context());
        self.set_data_on_pins(data);
    }

    /// Execute the CRTC register read (read of &BF00).
    pub fn read_crtc_register(&mut self) {
        let data = self.model.read_register(&self.read_context());
        self.set_data_on_pins(data);
    }

    /// What a register read may depend on. Advances the noise source, as a
    /// floating bus would return something different each time.
    fn read_context(&mut self) -> ReadContext<'_> {
        // xorshift32
        self.noise ^= self.noise << 13;
        self.noise ^= self.noise >> 17;
        self.noise ^= self.noise << 5;
        ReadContext {
            registers: &self.regs,
            selected: self.ar,
            vertical_border: self.vertical_border,
            noise: self.noise & 1 != 0
        }
    }

    /// Whether register `r` can be written on this CRTC (R0-R15; R16/R17
    /// are the read-only light pen).
    pub fn is_register_writable(&self, r: u8) -> bool {
        r <= 15
    }

    /// Store `v` in register `r`, keeping only the bits this chip has.
    /// Registers that don't exist are ignored.
    pub fn do_write_register(&mut self, r: u8, v: u8) {
        if let Some(log) = self.write_log.as_mut() {
            log.push((self.hcc, self.vcc.value(), self.vlc.value(), r, v));
        }
        // ACCC §13.2.1: C9 is no longer managed while R0=0, and R4, R5 and R9
        // updates are ignored with it (R8 still is)
        if self.model.has_last_line_state() && self.regs[0] == 0 && matches!(r, 4 | 5 | 9) {
            return;
        }
        let mask = self.model.write_mask(r);
        let previous_r3l = self.regs[3] & 0x0f;
        if let Some(register) = self.regs.get_mut(usize::from(r)) {
            *register = v & mask;
        }
        // R3.JIT, ACCC §14.4.4 / §14.5.4: R3l written (by an OUT(C),r8, on CRTC
        // 0, 1 and 2) with exactly the running C3l, while it was above it,
        // stops the HSYNC at once instead of letting the 4-bit counter wrap.
        if r == 3 && self.hsync() && self.model.supports_r3_jit() {
            let new_r3l = self.regs[3] & 0x0f;
            if new_r3l == self.hsc && previous_r3l > self.hsc {
                self.off(CrtcPin::HS);
            }
        }
        // ACCC §18.2.2: C4=R6 is considered immediately, whatever C0 (not on
        // the frame's first line, where R6 conflicts with C4=0, §18.3).
        if r == 6 && self.model.latches_r6_border() && self.vcc == self.regs[6]
            && !(self.vcc == 0 && self.vlc == 0)
        {
            self.r6_border = true;
            self.set_pin(CrtcPin::DE, false);
        }
        // ACCC §17.4: "the condition C0=R1 is considered immediately on a
        // line", and it can be met several times in one line by reprogramming
        // R1: the video pointer is updated (on the last scan line of the
        // row) and the BORDER starts at once.
        if r == 1 && self.hcc == self.regs[1] && (self.regs[1] != 0 || self.hcc != 0 || self.model.acknowledges_r1_zero_on_c0_zero()) {
            if self.vlc == self.regs[9] {
                self.crtc_vma_prime = self.crtc_vma;
            }
            self.border_history[0] = true;
            if let DisplaySkew::Delayed(chars) = self.model.display_skew(self.regs[8]) {
                if self.border_history[usize::from(chars)] {
                    self.set_pin(CrtcPin::DE, false);
                }
            }
        }
        // CRTC 0 keeps assessing the last line while C0<2 (ACCC 12.2)
        if (r == 4 || r == 9) && self.hcc < 2 {
            self.assess_last_line();
        }
        if r == 5 && self.hcc <= 2 {
            self.assess_additional_line();
        }
        // R2.JIT, ACCC §14.7.1: "if the update of R2 occurs while C0=R2
        // (during the 3rd usec of an OUT(C),r8) then the CRTC sends the HSYNC
        // signal later to the Gate Array". The compare of C0 with R2 already
        // went by at the start of this character; the HSYNC it would have
        // raised starts now, part-way through it.
        if r == 2 && self.hcc == self.regs[2] && !self.hsync() && self.hsync_width().is_some() {
            self.hsc = 0;
            self.on(CrtcPin::HS);
            self.hdc = 0;
        }
    }

    /// Start recording the register writes (diagnostics).
    pub fn start_write_log(&mut self) {
        self.write_log = Some(Vec::new());
    }

    /// The writes recorded since the last call, as (C0, C4, C9, register, value).
    pub fn take_write_log(&mut self) -> Vec<(u8, u8, u8, u8, u8)> {
        self.write_log.as_mut().map(std::mem::take).unwrap_or_default()
    }

    /// The register the next data write goes to (diagnostics).
    pub fn selected_register(&self) -> u8 {
        self.ar
    }

    /// ACCC 17.6.2: on this character (C0=R0 reached with the display still
    /// on) only the first of its two bytes carries video, the second is
    /// BORDER (CRTC 0 and 2, without an R8 display skew).
    pub fn second_byte_is_border(&self) -> bool {
        let skew = (self.regs[8] >> 4) & 3;
        self.model.anticipates_border_byte()
            && self.hcc == self.regs[0]
            && self.get_pin(CrtcPin::DE)
            && (skew == 0 || skew == 3)
    }

    /// ACCC 18.3.2: on the first line of a frame with R6=0, CRTC 0 and 2
    /// alternate a byte of video and a byte of BORDER (the second half of
    /// each character is BORDER) as long as R1 has not been reached.
    pub fn r6_conflict_second_byte_is_border(&self) -> bool {
        self.model.latches_r6_border()
            && self.vcc == 0
            && self.vlc == 0
            && self.regs[6] == 0
            && self.get_pin(CrtcPin::DE)
    }

    /// The CRTC 0 "last line of the frame" state (diagnostics).
    pub fn last_line(&self) -> bool {
        self.last_line
    }

    /// ACCC 12.2: while C0<2, CRTC 0 decides whether this is the frame's
    /// last line; it no longer looks at R4/R9 afterwards.
    fn assess_last_line(&mut self) {
        if self.model.has_last_line_state() && !self.in_vertical_adjust {
            self.last_line = self.vcc == self.regs[4] && self.vlc == self.regs[9];
            self.additional_armed = self.last_line;
            self.additional_r5 = self.last_line && self.regs[5] > 0;
        }
    }

    /// ACCC 13.2.1: C0=2 on the last line decides the additional lines; an R5
    /// programmed before C0=3 asks for them.
    fn assess_additional_line(&mut self) {
        if self.model.has_last_line_state() && self.last_line && !self.in_vertical_adjust {
            if self.hcc == 2 {
                self.additional_armed = false;
            }
            if self.hcc <= 2 {
                self.additional_r5 = self.regs[5] > 0;
            }
        }
    }

    /// The register as stored (no chip-dependent read behaviour: for that
    /// see [`Crtc::read_crtc_register`]).
    pub fn do_read_register(&self, r: u8) -> u8 {
        self.regs.get(usize::from(r)).copied().unwrap_or(0xff)
    }

    /// Which chip this is.
    pub fn crtc_type(&self) -> CrtcType {
        self.model.crtc_type()
    }

    /// R12/R13 as a 14-bit address (ACCC §20.1: "these two registers allow
    /// for the definition... of the base memory address").
    fn base_address(&self) -> u16 {
        (((self.regs[12] as u16) << 8).wrapping_add(self.regs[13] as u16)) & 0x3fff
    }

    /// ACCC §17.4/§20.3: at the start of every line (C0=0), reload the live
    /// `crtc_vma` from `crtc_vma_prime` - except the frame's very first
    /// line, which every CRTC type special-cases (called only once C0 has
    /// just wrapped to 0, from [`Crtc::update_memory_address`]).
    fn reload_vma_if_needed(&mut self) {
        if self.hcc != 0 {
            return;
        }
        if self.model.reloads_start_address_every_line_of_first_row() {
            // CRTC 1 (ACCC §17.4.2): while C4=0, VMA *itself* (not VMA') is
            // reloaded from R12/R13 directly, line by line - the offset can
            // be changed on each of that row's scan lines, not just once.
            if self.vcc == 0 {
                self.crtc_vma = self.base_address();
                return;
            }
        }
        else if self.vcc == 0 && self.vlc == 0 {
            // CRTC 0, 2, 3, 4 (ACCC §20.3/§17.4.1): only the frame's very
            // first line refreshes VMA' from R12/R13 before handing it to
            // VMA - every other line just replays whatever VMA' was left
            // holding from the end of the previous row.
            self.crtc_vma_prime = self.base_address();
        }
        self.crtc_vma = self.crtc_vma_prime;
    }

    /// VSync in lines
    fn vsync_height(&self) -> u8 {
        self.model.vsync_width(self.regs[3])
    }

    /// HSync width in chars; `None` when no HSYNC is produced.
    fn hsync_width(&self) -> Option<u8> {
        self.model.hsync_width(self.regs[3])
    }

    /// Run all the CRTC machinery that compute the next CRTC address
    fn update_memory_address(&mut self) {
        if self.hsync() {
            // ACCC §14.4: C3l is a 4-bit counter. Lowering R3l below it
            // during HSYNC (R3.JIT) makes it overflow and reach R3l on
            // the next lap (R3l=0 included: "0 is treated as a value to
            // reach"), except on CRTC 1 where R3l=0 cancels the HSYNC.
            self.hsc = (self.hsc + 1) & 0x0f;
            let r3l = self.regs[3] & 0x0f;
            if self.hsc == r3l || (r3l == 0 && self.model.zero_r3_cancels_running_hsync()) {
                self.off(CrtcPin::HS);
            }
        }

        self.hcc = self.hcc.wrapping_add(1); // XXX check if there is a bit limit in the counter
        self.hdc = self.hdc.wrapping_add(1);

        // CRTC-VMA (ACCC §17.3): a real, incrementing counter, advanced
        // once per character whatever the display state - not recomputed
        // from vcc/hcc, so a mid-frame R1/R12/R13 change only affects rows
        // from that point on, matching real hardware (see its own doc).
        self.crtc_vma = self.crtc_vma.wrapping_add(1) & 0x3fff;

        // ACCC §17.1/§17.4: latch where the *next* row starts, the moment
        // this row's active portion ends (C0=R1) on its last scan line
        // (C9=R9). Using `==` rather than `>=` is deliberate: it's what
        // lets a mid-line R1 rewrite (ACCC §17.3, "dynamic R1 update") make
        // this condition true a second time on the same line.
        // The counter does not reach R0+1: it wraps to 0 instead (ACCC
        // §17.2.2: with R1>R0, C0 is never equal to R1 and VMA' is stuck).
        if self.hcc == self.regs[1]
            && self.hcc != self.regs[0].wrapping_add(1)
            && self.vlc == self.regs[9]
        {
            self.crtc_vma_prime = self.crtc_vma;
        }

        if self.hcc == self.regs[0].wrapping_add(1) {
            // This is the end of the line, we have to trigger all the other stuffs
            self.hcc = 0;

            // handle the end of vsync if any - independent of vertical
            // total adjust, since VSYNC width (R3) and the adjust period
            // (R5) are unrelated and can overlap.
            if self.vsync() {
                self.vsc += 1;
                if self.vsc == self.vsync_height() {
                    self.off(CrtcPin::VS);
                }
            }

            // ACCC §13.2.1/§13.2.4: with R0=0 on CRTC 0, C0 never reaches 1 and
            // C9 stops being managed: the first wrap to C0=0 updates the
            // counters as usual (and arms C4 if C9 reached R9), the second
            // only moves C4 once, then everything is frozen.
            let frozen_r0 = self.model.has_last_line_state() && self.regs[0] == 0;
            let skip_counters = frozen_r0 && self.r0_zero_wraps >= 1;
            if frozen_r0 {
                if self.r0_zero_wraps == 1 && self.r0_zero_c4_pending {
                    self.vcc += 1;
                    self.r0_zero_c4_pending = false;
                }
                self.r0_zero_wraps = self.r0_zero_wraps.saturating_add(1);
            } else {
                self.r0_zero_wraps = 0;
                self.r0_zero_c4_pending = false;
            }
            if skip_counters {
                // frozen
            } else if self.in_vertical_adjust {
                // VCC/VLC are held here; only the adjust counter advances,
                // one raster line at a time, until it has topped up the
                // frame by R5 extra lines.
                self.vtac += 1;
                if self.vtac >= self.regs[5] {
                    self.in_vertical_adjust = false;
                    self.last_line = false;
                    self.vtac = 0;
                    self.vcc.reset();
                    self.vlc.reset();
                }
            } else if self.model.has_last_line_state() {
                // ACCC 12.2 / 10.3.1: the latched last-line state, or else
                // C9==R9 at this very moment decides everything
                if self.last_line {
                    self.last_line = false;
                    self.vlc.reset();
                    self.vcc += 1;
                    if !(self.additional_armed || self.additional_r5) {
                        self.vcc.reset();
                    } else {
                        self.in_vertical_adjust = true;
                        self.vtac = 0;
                    }
                } else if self.vlc == self.regs[9] {
                    self.vlc.reset();
                    self.vcc += 1;
                } else {
                    self.vlc += 1;
                }
            } else {
                self.vlc += 1;


                // This is the end of a char line
                if self.vlc == self.regs[9] + 1 {
                    self.vlc.reset();
                    self.vcc += 1;

                    if self.vcc == self.regs[4] + 1 {
                        if self.regs[5] == 0 {
                            // This is the end of the screen
                            self.vcc.reset();
                        } else {
                            // Enter vertical total adjust instead of
                            // immediately starting the next frame.
                            self.in_vertical_adjust = true;
                            self.vtac = 0;
                        }
                    }
                }
            }
            if frozen_r0 && self.r0_zero_wraps == 1 {
                self.r0_zero_c4_pending = !self.in_vertical_adjust && self.vlc == self.regs[9];
            }

            // Sampled once per line, at C0=R0 (ACCC §21.3.3); an R6 of 0
            // while the frame is running is not detected by the chip.
            self.vertical_border = self.regs[6] != 0 && self.vcc >= self.regs[6];
        }
        if self.hcc < 2 {
            self.assess_last_line();
        }
        self.assess_additional_line();

        // ACCC §19.2.3/§19.2.4: the horizontal BORDER condition (C0 reached
        // R1 - or R0 if R1 was not reached, CRTC 0 and 2) goes through a delay
        // line of 0 to 2 characters set by the R8 skew field.
        let skew = self.model.display_skew(self.regs[8]);
        let replaced_by_r0 = self.model.anticipates_border_byte()
            && self.hcc == self.regs[0]
            && matches!(skew, DisplaySkew::Delayed(d) if d > 0);
        self.border_history.rotate_right(1);
        self.border_history[0] = self.hcc >= self.regs[1] || replaced_by_r0;
        let horizontal_border = match skew {
            DisplaySkew::Delayed(chars) => self.border_history[usize::from(chars)],
            DisplaySkew::Blanked => true,
        };
        let vertical_border = if self.model.latches_r6_border() {
            let first_line = self.vcc == 0 && self.vlc == 0;
            if self.hcc == 0 && first_line {
                self.r6_border = false;
                self.r6_zero_seen = false;
            }
            if self.vcc == self.regs[6] {
                // ACCC §18.3.2: with C4=R6=C9=0 the BORDER R6 comes and goes
                // at each character; it only becomes final once C0=R1
                if first_line {
                    self.r6_zero_seen = true;
                } else {
                    self.r6_border = true;
                }
            }
            if self.r6_zero_seen && first_line && self.hcc >= self.regs[1] {
                self.r6_border = true;
            }
            self.r6_border
        } else {
            self.vcc >= self.regs[6]
        };
        if horizontal_border || vertical_border {
            self.set_pin(CrtcPin::DE, false); // Activate Border
        } else {
            // TODO handle the special case of CRTC1 that is unable to leave this state at some point
            self.set_pin(CrtcPin::DE, true);
        }

        // Request an hsync if needed (a zero width produces none on the
        // chips that treat it that way: it would otherwise never end)
        // ACCC 15.1: during a HSYNC the C0=R2 condition is not managed, so a
        // HSYNC cannot restart within a HSYNC
        if self.hcc == self.regs[2] && !self.hsync() && self.hsync_width().is_some() {
            self.hsc = 0;
            self.on(CrtcPin::HS);
            self.hdc = 0; // XXX I am pretty sure I am not reseting it at the right place
        }

        // Request a vsync if needed
        if self.vcc == self.regs[7] && !self.get_pin(CrtcPin::VS){
            self.vsc = 0;
            self.on(CrtcPin::VS); // Activate Vsync
        }

        self.reload_vma_if_needed();
        self.set_address_on_pins(); // set the address in the pins
    }

    /// MA and RA are variables manipulated during the process
    /// they are directly wired to the pin; this fonction replicate them here
    /// should not be called outside of Crtc. Public only for test reasons
    pub fn set_address_on_pins(&mut self) {
        self.set_value_in_pins(
            self.crtc_vma,
            &[
                CrtcPin::MA13,
                CrtcPin::MA12,
                CrtcPin::MA11,
                CrtcPin::MA10,
                CrtcPin::MA9,
                CrtcPin::MA8,
                CrtcPin::MA7,
                CrtcPin::MA6,
                CrtcPin::MA5,
                CrtcPin::MA4,
                CrtcPin::MA3,
                CrtcPin::MA2,
                CrtcPin::MA1,
                CrtcPin::MA0,
            ],
        );

        self.set_value_in_pins(
            self.vlc.value(),
            &[
                CrtcPin::RA4,
                CrtcPin::RA3,
                CrtcPin::RA2,
                CrtcPin::RA1,
                CrtcPin::RA0,
            ],
        )
    }

    /// Seed `crtc_vma`/`crtc_vma_prime` as if `row` whole character rows had
    /// already played out from `base_address` with R1 held constant the
    /// whole time - a shortcut for tests that want to start mid-frame (at
    /// the current `hcc`/`vlc`) without actually driving every tick of the
    /// rows before it. Real hardware, and the rest of this module, only
    /// ever reaches these values incrementally, tick by tick, through
    /// [`Crtc::reload_vma_if_needed`] - this is not a path either of them
    /// takes. Public only for test reasons.
    pub fn seed_vma_for_test(&mut self, base_address: u16, row: u16) {
        let stride = self.regs[1] as u16;
        let row_start = base_address.wrapping_add(row.wrapping_mul(stride)) & 0x3fff;
        self.crtc_vma = row_start.wrapping_add(self.hcc as u16) & 0x3fff;
        self.crtc_vma_prime = row_start.wrapping_add(stride) & 0x3fff;
    }
}

impl Crtc {
    /// Check if the CRTC as generated a VSYNC
    pub fn vsync(&self) -> bool {
        self.get_pin(CrtcPin::VS)
    }

    /// Check if the CRTC as generated a HSYNC
    pub fn hsync(&self) -> bool {
        self.get_pin(CrtcPin::HS)
    }

    ///  	The Video display address. This is derived from the MA register in the CRTC, the horizontal character count HCC and the current row address (RA) register in the CRTC. This is effectively the CPC memory address of the data displayed on the monitor.
    pub fn vma(&self) -> u16 {
        self.get_address_from_pins()
    }
}
impl Chip for Crtc {
    type PinType = CrtcPin;

    fn pins_buffer_mut(&mut self) -> &mut PinsBuffer {
        &mut self.pins
    }
    fn pins_buffer(&self) -> &PinsBuffer {
        &self.pins
    }

    fn must_execute_tick(&self) -> bool {
        !self.get_pin(CrtcPin::CS)  & !self.get_pin(CrtcPin::EN)
    }

    /// Execute the requested command from the CPU
    fn execute_tick(&mut self) {

        match (self.get_pin(CrtcPin::RW), self.get_pin(CrtcPin::RS)) {
            (false, false) => self.select_crtc_register(),
            (false, true) => self.write_crtc_register(),
            (true, false) => self.status_register(),
            (true, true) => self.read_crtc_register(),
        };
    }

    /// Execute the normal CRTC address behavior
    fn pre_tick(&mut self) {
        self.update_memory_address();
    }

    fn reset(&mut self) {
        self.crtc_vma = 0;
        self.crtc_vma_prime = 0;

        self.regs[0] = 63;
        self.regs[1] = 40;
        self.regs[2] = 46;
        self.regs[3] = 128 + 14;
        self.regs[4] = 38;
        self.regs[5] = 0;
        self.regs[6] = 25;
        self.regs[7] = 30;
        self.regs[8] = 0;
        self.regs[9] = 7;
        self.regs[10] = 0;
        self.regs[11] = 0;
        self.regs[12] = 48;
        self.regs[13] = 0;
        self.regs[14] = 0;
        self.regs[15] = 0;
        self.regs[16] = 0;
        self.regs[17] = 0;

        // Set the counters to the limits
        self.vcc = self.regs[4].into();
        self.vlc = self.regs[9].into();
        self.hcc = self.regs[0];
        self.vtac = 0;
        self.in_vertical_adjust = false;
        // the counters sit on the last line of the frame
        self.last_line = self.model.has_last_line_state();

        self.set_pin(CrtcPin::DE, true);
        self.set_pin(CrtcPin::HS, false);
        self.set_pin(CrtcPin::VS, false);
    }
}



impl DataChip for Crtc {
    fn pins_data() -> &'static [Self::PinType; 8] {
        return &CRTC_PIN_DATA_BUS;
    }
}
impl ReadDataChip for Crtc {}
impl WriteDataChip for Crtc {}

impl AddressChip for Crtc {
    fn pins_address() -> &'static [Self::PinType; 16] {
        return &CRTC_PIN_ADDRESS_BUS;
    }
}
impl ReadAddressChip for Crtc {
    /// CCLK as to be negated
    fn get_address_from_pins(&self) -> u16 {
        let addr = self.get_value_from_pins(Self::pins_address());
        addr ^ 1
    }
}
// Does not implement the writable one because adress is generated
impl Activable for Crtc {

    fn activated_at_up_front() -> bool {
        false
    }
    fn clock_pin() -> Self::PinType {
        CrtcPin::CCLK
    }
}

#[cfg(test)]
mod test {
    use super::*;

 //   #[test]
 //   fn test_chip_selection() {
 //       unimplemented!()

    #[test]
    fn test_address_generation_0xc000() {
        let mut crtc = Crtc::default();
        crtc.reset();

        assert_eq!(crtc.regs[1], 40);
        assert_eq!(crtc.regs[4], 38);
        assert_eq!(crtc.regs[9], 7);

        crtc.set_pin(CrtcPin::CCLK, true);
        crtc.set_pin(CrtcPin::CS, false);
        crtc.set_pin(CrtcPin::RW, false);
        crtc.set_pin(CrtcPin::RS, false);
        crtc.set_pin(CrtcPin::EN, false);

        crtc.regs[12] = 0x30;
        crtc.regs[13] = 0x00;
        crtc.vcc.reset();
        crtc.vlc.reset();
        crtc.hcc = 0;
        crtc.crtc_vma = 0x3000;
        crtc.set_address_on_pins();

        let address1 = crtc.get_address_from_pins();

        crtc.set_pin(CrtcPin::CCLK, false);
        crtc.set_address_on_pins();

        let address2 = crtc.get_address_from_pins();

        assert_eq!((0xc000, 0xc001), (address1, address2));
    }

    #[test]
    fn test_address_generation_0x0000() {
        let mut crtc = Crtc::default();
        crtc.reset();

        assert_eq!(crtc.regs[1], 40);
        assert_eq!(crtc.regs[4], 38);
        assert_eq!(crtc.regs[9], 7);

        crtc.set_pin(CrtcPin::CCLK, true);
        crtc.set_pin(CrtcPin::CS, false);
        crtc.set_pin(CrtcPin::RW, false);
        crtc.set_pin(CrtcPin::RS, false);
        crtc.set_pin(CrtcPin::EN, false);

        crtc.regs[12] = 0x00;
        crtc.regs[13] = 0x00;
        crtc.vcc.reset();
        crtc.vlc.reset();
        crtc.hcc = 0;
        crtc.crtc_vma = 0x0000;
        crtc.set_address_on_pins();

        let address1 = crtc.get_address_from_pins();

        crtc.set_pin(CrtcPin::CCLK, false);
        crtc.set_address_on_pins();

        let address2 = crtc.get_address_from_pins();

        assert_eq!((0x0000, 0x0001), (address1, address2));
    }

    /// Test the inner working of crtc for a standard screen
    #[test]
    fn test_standard_crtc() {
        let mut crtc = Crtc::default();

        for char_line in 0..=crtc.regs[4] {
            for scan_line in 0..=crtc.regs[9] {
                for char in 0..=crtc.regs[0] {
                    crtc.save_pins_state();
                    crtc.set_pin(CrtcPin::CCLK, true);

                    assert!(!crtc.is_activated());

                    crtc.save_pins_state();
                    crtc.set_pin(CrtcPin::CCLK, false);
                    assert!(crtc.is_activated());
                    assert!(crtc.must_execute_tick());

                    // execute the CRTC address computation
                    crtc.tick();

                    //  dbg!(char_line, scan_line, char, &crtc);

                    // Check that main internal counter are correct
                    assert!(crtc.vcc <= crtc.regs[4]);
                    assert!(crtc.vlc.value() <= crtc.regs[9]);
                    assert!(crtc.hcc <= crtc.regs[0]);

                    assert!(crtc.vcc == char_line);
                    assert!(crtc.vlc.value() == scan_line);
                    assert!(crtc.hcc == char);

                    // Test for border
                    if crtc.hcc >= crtc.regs[1] || crtc.vcc >= crtc.regs[6] {
                        assert!(!crtc.get_pin(CrtcPin::DE));
                    } else {
                        assert!(crtc.get_pin(CrtcPin::DE));
                    }

                    // Test for hsync
                    if crtc.hcc >= crtc.regs[2] && crtc.hcc < crtc.regs[2] + crtc.hsync_width().unwrap() {
                        assert!(crtc.get_pin(CrtcPin::HS));
                    } else {
                        assert!(!crtc.get_pin(CrtcPin::HS));
                    }

                    // VSYNC duration is expressed in raster lines, not in character lines.
                    let raster_line = (crtc.vcc.value() as u16) * (crtc.regs[9] as u16 + 1)
                        + crtc.vlc.value() as u16;
                    let vsync_start = (crtc.regs[7] as u16) * (crtc.regs[9] as u16 + 1);
                    let vsync_end = vsync_start + crtc.vsync_height() as u16;
                    if raster_line >= vsync_start && raster_line < vsync_end {
                        assert!(crtc.get_pin(CrtcPin::VS));
                    } else {
                        assert!(!crtc.get_pin(CrtcPin::VS));
                    }

                    // test for address
                    if crtc.get_pin(CrtcPin::DE) {
                        let expected = 0xc000
                            + 0x800 * (scan_line as u16)
                            + 80 * (char_line as u16)
                            + 2 * (char as u16)
                            + 1;
                        let collected = crtc.get_address_from_pins();

                        assert_eq!(expected, collected, "0x{:x}!=0x{:x}", expected, collected);
                    }
                }
            }
        }
    }

    #[test]
    fn test_hsync() {
        let mut crtc = Crtc::default();

        crtc.update_memory_address();

        // ensure hsync is not present
        while !crtc.hsync() | crtc.vsync() {
            crtc.update_memory_address();
        }

        for i in 0..2 {
            let mut hsync_duration = 0;
            while crtc.hsync() {
                hsync_duration += 1;
                crtc.update_memory_address();
            }
            assert_eq!(Some(hsync_duration), crtc.hsync_width());

            let mut hsync_wait = 0;
            while !crtc.hsync() {
                hsync_wait += 1;
                crtc.update_memory_address();
            }
            assert_eq!(hsync_duration + hsync_wait, 64);
        }
    }

    /// ACCC §14.4: lowering R3l below the running 4-bit HSYNC counter makes
    /// it overflow, so the HSYNC lasts until the counter reaches R3l again
    /// (16 - old + new chars from its start), not 256 chars.
    #[test]
    fn test_hsync_r3_lowered_during_hsync_overflows() {
        let mut crtc = Crtc::default();
        crtc.update_memory_address();
        while !crtc.hsync() {
            crtc.update_memory_address();
        }
        // let the counter run past the new value, then lower R3l below it
        for _ in 0..5 {
            crtc.update_memory_address();
        }
        assert!(crtc.hsync());
        crtc.regs[3] = (crtc.regs[3] & 0xf0) | 2;
        let mut extra = 0;
        while crtc.hsync() {
            extra += 1;
            assert!(extra <= 16, "HSYNC never ended");
            crtc.update_memory_address();
        }
        assert!(extra > 5, "HSYNC must overflow the 4-bit counter, got {extra}");
    }

    #[test]
    fn test_vsync() {
        let mut crtc = Crtc::default();

        crtc.update_memory_address();

        // ensure hsync is not present
        while !crtc.vsync() {
            crtc.update_memory_address();
        }

        for i in 0..2 {
            let mut vsync_duration = 0;
            while crtc.vsync() {
                vsync_duration += 1;
                crtc.update_memory_address();
            }

            let mut vsync_wait = 0;
            while !crtc.vsync() {
                vsync_wait += 1;
                crtc.update_memory_address();
            }
            assert_eq!(vsync_duration + vsync_wait, 19968);
        }
    }
}
