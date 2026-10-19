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

/// C0 below which an R7 written with the value of C4 does not start a VSYNC
const BLOCKED_R7_C0: u8 = 2;

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
    /// ACCC §13.7.2: C4 steps at C0=2 after an R0 enlargement out of 1.
    r0_enlarge_steps_c4: bool,
    /// ACCC §13.7.2.2: C9 counts on from the last line up to R5.
    r0_enlarge_c9_to_r5: bool,
    /// The running adjustment was asked for by R5>0 (CRTC 0: it ends when the
    /// count equals R5, read live) and not by the additional management alone.
    adjust_counts_to_r5: bool,
    /// ACCC 16.4.1.1: an R7 written with C4's value on C0<2 is a BLOCKED VSYNC (CRTC 0):
    /// no VSYNC until C4 or R7 changes. (ACCC 16.3's wider "no second VSYNC while the
    /// equality is unchanged" made Shaker CT worse against AMSpiriT, so it is not applied.)
    r7_match: bool,
    /// Characters left of the blanking window of an R3l=2 HSYNC, which the Gate Array draws a character
    /// before the CRTC's pulse (see `hsync_blank_window`)
    early_blank_chars: u8,
    /// the running HSYNC began as an R3l=2 one (its character-early blanking window was opened)
    two_wide_pulse: bool,
    /// ACCC 16.4.1: a CRTC 0 VSYNC begun in the middle of a line does not count that line
    vsync_skips_first_line_end: bool,
    /// The VSYNC ended while C4 still equals R7: it does not start again
    /// before C4 moves (next row or frame) or R7 changes (AMSpiriT, CPCSE)
    vsync_ended_on_r7: bool,
    /// The VSYNC runs (counted by C3h). The VS pin follows it, except for a ghost VSYNC
    /// (see [`CrtcModel::ghost_vsync_during_hsync`]).
    vsync_running: bool,
    /// CRTC 0 (ACCC §13.2.2): the line reached C0=2, which validates the VSYNC test of the
    /// next C0=0 (cancelled there): with R0<2 no VSYNC starts
    vsync_authorized: bool,
    /// Number of line ends (C0 back to 0 through R0) since reset; for tests and tools
    pub line_ends: u64,
    /// Motherboard ticks since the last character clock edge (0..15).
    phase_ticks: u8,
    /// BORDER R1 latched on this line (CRTC 0/2), see `update_memory_address`.
    r1_latch: bool,
    /// R1=0 came after C0=0: not considered until the next line.
    r1_zero_late: bool,
    /// How many ticks late the running HSYNC rose compared to its C0=R2
    /// character (R2.JIT, ACCC 14.7.1: the monitor sync does not move).
    hsync_late_ticks: u8,
    /// The HSYNC ended as C0 reached its current value: C0=R2 is not looked at on that character
    /// (CRTC 0 only, Shaker AT E: R2 moved during the pulse to where it ends does not start another
    /// one; CRTCs 1-4 start it, AT E and BCOPY A there).
    hsync_ended_on_char: bool,
    /// CRTC 3/4: register writes waiting to land (ticks left, register, value).
    delayed_writes: Vec<(u8, u8, u8)>,
    /// CRTC 2 (ACCC 12.4.1): "Last Line" (C4 and C9 go to 0 on the next line, decided on C0==0 or by an
    /// R4/R9 update and never cancelled during the line), "Last Line Management" (an R4/R9 update may
    /// still set it: not on the frame's first line) and "Previous Last Line" (C4==R4 and C9==R9 on the
    /// last character of the HSYNC: the next C0==0 does not take the line as a last one again).
    c2_last_line: bool,
    c2_last_line_management: bool,
    c2_previous_last_line: bool,
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
    /// CRTC 0 interlace (ACCC §19): the parity of the frame (true = odd), the
    /// parity ParityR6 that anticipates the next frame's, the parity of the
    /// C9 lines being generated in interlace video mode, and whether the
    /// lines currently counted use the doubled C9 (R8=3).
    parity_frame: bool,
    parity_r6: bool,
    parity_c9: bool,
    ivm_lines: bool,
    /// An extra interlace line follows the R5 lines of this frame.
    interlace_extra_line: bool,
    /// CRTC 1 (ACCC 11.6): a rupture for dummies is running: VMA is reloaded from R12/R13 at the
    /// start of every line until the C9=R9 test at C0=R1 passes (it depends on the frame parity)
    rfd_active: bool,
    /// CRTC 1: the address VMA' will take at the end of this character if C0=R1 still holds
    vma_latch_pending: Option<u16>,
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
            r0_enlarge_steps_c4: false,
            r0_enlarge_c9_to_r5: false,
            adjust_counts_to_r5: false,
            r7_match: false,
            early_blank_chars: 0,
            two_wide_pulse: false,
            vsync_skips_first_line_end: false,
            vsync_ended_on_r7: false,
            vsync_running: false,
            vsync_authorized: true,
            line_ends: 0,
            phase_ticks: 0,
            r1_latch: false,
            r1_zero_late: false,
            hsync_late_ticks: 0,
            hsync_ended_on_char: false,
            delayed_writes: Vec::new(),
            c2_last_line: false,
            c2_last_line_management: true,
            c2_previous_last_line: false,
            write_log: None,
            crtc_vma: 0,
            crtc_vma_prime: 0,
            model: CrtcType::default().model(),
            vertical_border: false,
            border_history: [false; 3],
            r6_border: false,
            r0_zero_wraps: 0,
            parity_frame: false,
            parity_r6: false,
            parity_c9: false,
            ivm_lines: false,
            interlace_extra_line: false,
            rfd_active: false,
            vma_latch_pending: None,
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

/// CRTC 3/4 (the ASICs): a register write lands this many ticks (1 usec) after the I/O cycle.
const ASIC_WRITE_DELAY_TICKS: u8 = 16;

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

    /// Execute the CRTC register write. The ASICs (CRTC 3 and 4) take it 1 usec later (ACCC 13.5: "the
    /// I/O takes place with 1 us delay, which implies placing the OUT 1 usec before those that would be
    /// done on a CRTC 1").
    pub fn write_crtc_register(&mut self) {
        let val = self.get_data_from_pins();
        if self.is_register_writable(self.ar) {
            // (R3 excepted: the ASIC's HSYNC already comes a character late, see `CrtcModel`; delaying
            // R3 too puts Shaker AY F and BI B-D on CRTC 4 away from the real CPC)
            if matches!(self.crtc_type(), CrtcType::Crtc3 | CrtcType::Crtc4) && self.ar != 3 {
                self.delayed_writes.push((ASIC_WRITE_DELAY_TICKS, self.ar, val));
            } else {
                self.do_write_register(self.ar, val);
            }
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
        {
            use std::sync::OnceLock;
            static TRACE: OnceLock<bool> = OnceLock::new();
            if *TRACE.get_or_init(|| std::env::var("AMSRUST_TRACE_REG").is_ok()) {
                eprintln!("W R{r}<-{v} C0={} C4={} C9={}", self.hcc, self.vcc.value(), self.vlc.value());
            }
        }
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
        let previous_r5 = self.regs[5];
        let previous_r8 = self.regs[8];
        let previous_r0 = self.regs[0];
        // ACCC 16.4.1.1: R7 given the value of C4 on C0=0/1 is a BLOCKED VSYNC: it
        // does not start, nor can it until C4 or R7 changes
        if r == 7
            && self.model.has_last_line_state()
            && !self.vsync()
            && self.vcc.value() == v & mask
            && self.vcc.value() != self.regs[7]
            && self.hcc < BLOCKED_R7_C0
        {
            self.r7_match = true;
        }
        if let Some(register) = self.regs.get_mut(usize::from(r)) {
            *register = v & mask;
        }
        if self.crtc_type() == CrtcType::Crtc2 {
            self.crtc2_register_update(r);
        }
        // ACCC §19.5.3 (CRTC 1): the parities follow R8 as soon as it is written
        if r == 8 && self.model.has_interlace_video() {
            let was_ivm = previous_r8 & 3 == 3;
            let is_ivm = self.regs[8] & 3 == 3;
            if was_ivm != is_ivm {
                let c4_odd_r9_even = self.vcc.value() & 1 != 0 && self.regs[9] & 1 == 0;
                let c9 = self.c9_vma();
                // 3rd microsecond: ParityC9 = C9.0 xor (C4.0 and not R9.0)
                let mut parity_c9 = (c9 & 1 != 0) ^ c4_odd_r9_even;
                if is_ivm {
                    // 4th microsecond: IVM becomes active
                    if !self.parity_frame {
                        parity_c9 = c4_odd_r9_even;
                    }
                    self.parity_frame = self.parity_frame && (parity_c9 ^ c4_odd_r9_even);
                    self.vlc = ((c9 & !1) | u8::from(parity_c9)).into();
                    self.parity_c9 = parity_c9;
                } else {
                    // IVM goes idle: ParityFrame = ParityC9, C9 keeps its bit 0 as ParityC9
                    self.parity_frame = parity_c9;
                    self.vlc = ((c9 & !1) | u8::from(parity_c9)).into();
                }
                self.ivm_lines = is_ivm;
            }
        }
        // ACCC §19.8.4 (CRTC 3/4): IVM switched on takes ParityC9 from the current C9
        if r == 8 && self.model.c9_counts_like_asic() && previous_r8 & 3 != 3 && self.regs[8] & 3 == 3 {
            self.parity_c9 = self.vlc.value() & 1 != 0;
        }
        // ACCC §11.6 (CRTC 1): R5 raised from 0 on C0=R0 starts a rupture for dummies
        if r == 5 && self.model.has_rfd() && previous_r5 & 0x1f == 0 && self.regs[5] & 0x1f != 0
            && self.hcc == self.regs[0]
        {
            self.rfd_active = true;
        }
        // ACCC §13.7.2: R0 enlarged out of 1 when C0=1 and C9=R9 overflows C4
        // (even if C4<>R4); the step itself lands at C0=2.
        if r == 0 && self.model.has_last_line_state() && self.hcc == 1 && previous_r0 == 1
            && self.regs[0] > 1 && self.c9_is_last() && !self.in_vertical_adjust
        {
            self.r0_enlarge_steps_c4 = true;
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
        // an early window of an R3l=2 HSYNC that turns out wide (R3 raised on the very
        // character R2 names, HBL's main sync) is not drawn early
        if r == 3 && self.early_blank_chars > 0 && self.hcc == self.regs[2] && !self.hsync_is_two_wide() {
            self.early_blank_chars = 0;
            self.two_wide_pulse = false;
        }
        // ACCC §18.2.2: C4=R6 is considered immediately, whatever C0 (not on
        // the frame's first line, where R6 conflicts with C4=0, §18.3).
        if r == 6 && self.model.latches_r6_border() && self.vcc == self.regs[6]
            && !(self.vcc == 0 && self.vlc == 0)
        {
            self.r6_border = true;
            self.set_pin(CrtcPin::DE, false);
        }
        // CRTC 1 (ACCC §18.3.3): R6=0 written on the first line of a frame (C4=R6=0)
        // makes the BORDER final for the rest of the frame
        if r == 6 && self.model.r6_zero_is_border() && self.regs[6] == 0 && self.vcc == 0 && self.vlc == 0 {
            self.r6_border = true;
            self.set_pin(CrtcPin::DE, false);
        }
        // ACCC §17.5: R1=0 written after C0=0 is too late for this line
        if r == 1 && self.regs[1] == 0 && self.hcc > 0 {
            self.r1_zero_late = true;
        }
        // ACCC §17.4: "the condition C0=R1 is considered immediately on a
        // line", and it can be met several times in one line by reprogramming
        // R1: the video pointer is updated (on the last scan line of the
        // row) and the BORDER starts at once.
        if r == 1 && self.hcc == self.regs[1] && (self.regs[1] != 0 || self.hcc != 0 || self.model.acknowledges_r1_zero_on_c0_zero()) {
            self.r1_reached();
            self.border_history[0] = true;
            self.r1_latch = true;
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
            self.hsync_late_ticks = self.phase_ticks;
        }
    }

    /// The motherboard tells where in the character clock this tick falls (every tick: the ASICs'
    /// delayed register writes land then).
    pub fn set_phase_ticks(&mut self, phase: u8) {
        self.phase_ticks = phase;
        if !self.delayed_writes.is_empty() {
            let mut i = 0;
            while i < self.delayed_writes.len() {
                self.delayed_writes[i].0 -= 1;
                if self.delayed_writes[i].0 == 0 {
                    let (_, r, v) = self.delayed_writes.remove(i);
                    self.do_write_register(r, v);
                } else {
                    i += 1;
                }
            }
        }
    }

    pub fn phase_ticks(&self) -> u8 {
        self.phase_ticks
    }

    /// Ticks by which the running HSYNC started late (R2.JIT), else 0.
    pub fn hsync_late_ticks(&self) -> u8 {
        self.hsync_late_ticks
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
    /// Register `r` as last written (0 past R17), for debuggers: unlike
    /// [`Self::do_read_register`], whatever the CRTC type lets the CPU read.
    pub fn register_value(&self, r: u8) -> u8 {
        self.regs.get(usize::from(r)).copied().unwrap_or(0)
    }

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
            && !self.model.r6_zero_is_border()
            && self.vcc == 0
            && self.vlc == 0
            && self.regs[6] == 0
            && self.get_pin(CrtcPin::DE)
    }

    /// The CRTC 0 "last line of the frame" state (diagnostics).
    pub fn last_line(&self) -> bool {
        self.last_line
    }

    /// CRTC 0, R8=1 or 3: the two interlace modes (ACCC §19.3).
    fn interlaced(&self) -> bool {
        self.model.supports_interlace() && self.regs[8] & 1 != 0
    }

    /// CRTC 0, R8=3: interlace sync and video mode.
    fn ivm(&self) -> bool {
        self.model.has_last_line_state() && self.regs[8] & 3 == 3
    }

    /// C9 as it builds the video address (ACCC §19.8.1): shifted left, bit 0
    /// carrying the parity, while the lines are counted in IVM.
    fn c9_vma(&self) -> u8 {
        if self.in_vertical_adjust && self.adjust_counts_to_r5 && !self.ivm_lines {
            // ACCC 11.2.2: on CRTC 0 there is no C5, C9 itself counts the lines of
            // the adjustment (and so goes out on RA)
            self.vtac & 0x1f
        } else if self.ivm_lines && !self.model.frame_parity_toggles() {
            (self.vlc.value() << 1 | u8::from(self.parity_c9)) & 0x1f
        } else {
            self.vlc.value()
        }
    }

    /// Register `r` as stored (tools)
    pub fn regs_r(&self, r: usize) -> u8 {
        self.regs[r]
    }

    /// The value of C9 as it goes out on RA0-RA4.
    pub fn c9(&self) -> u8 {
        self.c9_vma()
    }

    /// Whether C9 is on the last line of the character row.
    fn c9_is_last(&self) -> bool {
        if self.model.c9_counts_like_asic() {
            // CRTC 3/4 (ACCC 10.3.4, 19.8.4): the row ends on C9 >= R9, in IVM or not
            self.vlc.value() >= self.regs[9]
        } else if self.ivm_lines && self.model.frame_parity_toggles() {
            // CRTC 1 (ACCC 19.8.2): the test excludes bit 0
            self.vlc.value() & 0x1e == self.regs[9] & 0x1e
        } else if self.ivm_lines {
            let r9 = self.regs[9];
            self.c9_vma() == (r9 + ((r9 ^ u8::from(self.parity_c9)) & 1)) & 0x1f
        } else {
            self.vlc == self.regs[9]
        }
    }

    /// CRTC 2, C0==0 (ACCC 12.4.1): the line is a last line when C4==R4 and C9==R9 (the R9 of the
    /// previous line: an update on this C0 comes too late), unless the previous line already was one
    /// or an HSYNC is on C0==0, starting there or still running from the previous line (ACCC 15.4
    /// note 1, 15.6); R4/R9 updates may still make it one, except on a frame's first line.
    fn crtc2_line_start(&mut self) {
        self.c2_last_line = !self.in_vertical_adjust
            && self.vcc == self.regs[4]
            && self.vlc == self.regs[9]
            && !self.c2_previous_last_line
            && self.regs[2] != 0
            && !self.hsync();
        self.c2_last_line_management = !(self.vcc.value() == 0 && self.vlc.value() == 0);
    }

    /// CRTC 2: an R4/R9 update outside HSYNC can make the current line the last one (ACCC 12.4.1).
    fn crtc2_register_update(&mut self, r: u8) {
        if matches!(r, 4 | 9)
            && !self.c2_last_line
            && self.c2_last_line_management
            && !self.hsync()
            && !self.in_vertical_adjust
            && !(r == 9 && self.hcc == 0)
        {
            self.c2_last_line = self.vcc == self.regs[4] && self.vlc == self.regs[9];
        }
    }

    /// ACCC 12.2: while C0<2, CRTC 0 decides whether this is the frame's
    /// last line; it no longer looks at R4/R9 afterwards.
    fn assess_last_line(&mut self) {
        if self.model.has_last_line_state() && !self.in_vertical_adjust {
            self.last_line = self.vcc == self.regs[4] && self.c9_is_last();
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
            if self.vcc == 0 || self.rfd_active {
                self.crtc_vma = self.base_address();
                return;
            }
        }
        else if self.vcc == 0 && self.vlc == 0 && !self.model.frame_address_from_vma_prime() {
            // CRTC 0, 2, 3, 4 (ACCC §20.3/§17.4.1): only the frame's very
            // first line refreshes VMA' from R12/R13 before handing it to
            // VMA - every other line just replays whatever VMA' was left
            // holding from the end of the previous row.
            self.crtc_vma_prime = self.base_address();
        }
        self.crtc_vma = self.crtc_vma_prime;
    }

    /// C0=R1: VMA' takes the address of the next row on a row's last line, or R12/R13 on the
    /// frame's last line on CRTC 2 (see [`CrtcModel::frame_address_from_vma_prime`]).
    fn r1_reached(&mut self) {
        if self.model.frame_address_from_vma_prime() && self.is_frame_last_line() {
            self.crtc_vma_prime = self.base_address();
        } else if self.c9_is_last() {
            self.latch_next_row_address();
        }
    }

    /// The last line of the frame: C4=R4 and C9=R9, or the last line of the adjustment.
    fn is_frame_last_line(&self) -> bool {
        if self.in_vertical_adjust {
            self.vtac.saturating_add(1) >= self.regs[5].saturating_add(u8::from(self.interlace_extra_line))
        } else {
            self.vcc == self.regs[4] && self.c9_is_last()
        }
    }

    /// VMA' takes the address of the next row (C9=R9 at C0=R1). During an RFD (ACCC 11.6.1) the
    /// test is made with the frame parity: on an even frame it fails and the row repeats, on an
    /// odd one it passes, VMA' is updated and the RFD is over.
    fn latch_next_row_address(&mut self) {
        if self.model.r1_jit_cancels_vma_latch() {
            self.vma_latch_pending = Some(self.crtc_vma);
            return;
        }
        self.commit_next_row_address(self.crtc_vma);
    }

    fn commit_next_row_address(&mut self, vma: u16) {
        if self.rfd_active {
            if !self.parity_frame {
                return;
            }
            self.rfd_active = false;
        }
        self.crtc_vma_prime = vma;
    }

    /// C0 has gone past R0: the line is over (on CRTC 3/4 also when R0 was lowered below C0).
    fn line_total_reached(&self) -> bool {
        self.hcc == self.regs[0].wrapping_add(1)
            || (self.model.line_ends_past_r0() && self.regs[0] != 0xff && self.hcc > self.regs[0])
    }

    /// VSync in lines
    fn vsync_height(&self) -> u8 {
        self.model.vsync_width(self.regs[3])
    }

    /// `Some(blank)` while R3l=2: the Gate Array's blanking follows this window instead of the HSYNC pin
    pub fn hsync_blank_window(&self) -> Option<bool> {
        if self.early_blank_chars > 0 {
            Some(true)
        } else {
            (self.two_wide_pulse && self.hsync()).then_some(false)
        }
    }

    /// R3l=2 on a CRTC 0: its HSYNC is too short for the monitor (see the Gate Array's mode latch)
    pub fn hsync_is_two_wide(&self) -> bool {
        self.model.has_last_line_state() && self.hsync_width() == Some(2)
    }

    /// HSync width in chars; `None` when no HSYNC is produced.
    fn hsync_width(&self) -> Option<u8> {
        self.model.hsync_width(self.regs[3])
    }

    /// Run all the CRTC machinery that compute the next CRTC address
    fn update_memory_address(&mut self) {
        // CRTC 1 (ACCC 17.4.2): the character where C0=R1 has run its course: VMA' is updated only if
        // R1 still names it (no R1.JIT during it)
        if let Some(vma) = self.vma_latch_pending.take() {
            if self.hcc == self.regs[1] {
                self.commit_next_row_address(vma);
            }
        }
        let hsync_before = self.hsync();
        self.hsync_ended_on_char = false;
        if self.hsync() {
            // ACCC §14.4: C3l is a 4-bit counter. Lowering R3l below it
            // during HSYNC (R3.JIT) makes it overflow and reach R3l on
            // the next lap (R3l=0 included: "0 is treated as a value to
            // reach"), except on CRTC 1 where R3l=0 cancels the HSYNC.
            self.hsc = (self.hsc + 1) & 0x0f;
            // an R3l=2 written during the pulse (HBL's panel: R3=7 then 2 on C0=R2+1) makes it a
            // short pulse too, not blanked from then on - if it still ends at 2 (written later, C3l
            // has passed 2 and the pulse runs on: Shaker AY C keeps it black)
            if self.hsync_is_two_wide() && self.hsc <= 2 {
                self.two_wide_pulse = true;
            }
            let r3l = self.regs[3] & 0x0f;
            if self.hsc == r3l || (r3l == 0 && self.model.zero_r3_cancels_running_hsync()) {
                if self.crtc_type() == CrtcType::Crtc2 {
                    self.c2_previous_last_line = self.vcc == self.regs[4] && self.vlc == self.regs[9];
                    if !self.c2_previous_last_line {
                        self.c2_last_line_management = true;
                    }
                }
                self.off(CrtcPin::HS);
                self.hsync_ended_on_char = self.crtc_type() == CrtcType::Crtc0;
            }
        }

        self.early_blank_chars = self.early_blank_chars.saturating_sub(1);
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
        if self.hcc == self.regs[1] && !self.line_total_reached() {
            self.r1_reached();
        }

        if self.line_total_reached() {
            // This is the end of the line, we have to trigger all the other stuffs
            self.hcc = 0;
            self.line_ends += 1;

            // handle the end of vsync if any - independent of vertical
            // total adjust, since VSYNC width (R3) and the adjust period
            // (R5) are unrelated and can overlap.
            if self.vsync() {
                if self.vsync_skips_first_line_end {
                    self.vsync_skips_first_line_end = false;
                } else {
                    // a 4 bit counter: an R3 lowered below the count makes it go round
                    // (16 lines more), and a width of 16 is the counter back at 0
                    self.vsc = (self.vsc + 1) & 0x0f;
                    if self.vsc == self.vsync_height() & 0x0f {
                        self.vsync_running = false;
                        self.off(CrtcPin::VS);
                        self.vsync_ended_on_r7 = self.vcc == self.regs[7];
                    }
                }
            }

            // ACCC §13.2.1/§13.2.4: with R0=0 on CRTC 0, C0 never reaches 1 and
            // C9 stops being managed: the first wrap to C0=0 updates the
            // counters as usual (and arms C4 if C9 reached R9), the second
            // only moves C4 once, then everything is frozen.
            let frozen_r0 = self.model.has_last_line_state() && self.regs[0] == 0;
            let skip_counters = frozen_r0;
            if frozen_r0 {
                if self.r0_zero_wraps == 0 {
                    // First C0=0 with R0=0: C9 is compared with R9 for the first
                    // time; on C9=R9 it stays put and C4 moves on the next one
                    // (13.2.6), starting the additional management when C4=R4.
                    // Nothing moves at all while an adjustment is running (13.2.3).
                    let hiccup = !self.in_vertical_adjust && self.c9_is_last();
                    if !hiccup && !self.in_vertical_adjust {
                        self.vlc += 1;
                    }
                    self.r0_zero_c4_pending = hiccup;
                    if hiccup && self.vcc == self.regs[4] {
                        self.r0_enlarge_c9_to_r5 = true;
                    }
                } else if self.r0_zero_wraps == 1 && self.r0_zero_c4_pending {
                    self.vcc += 1;
                    self.r0_zero_c4_pending = false;
                }
                self.r0_zero_wraps = self.r0_zero_wraps.saturating_add(1);
            } else {
                self.r0_zero_wraps = 0;
                self.r0_zero_c4_pending = false;
            }
            let mut new_frame = false;
            let vcc_before = self.vcc.value();
            let adjust_before = self.in_vertical_adjust;
            if skip_counters {
                // frozen
            } else if self.r0_enlarge_c9_to_r5 {
                self.last_line = false;
                self.vlc += 1;
                if self.vlc.value() == self.regs[5] & 0x1f {
                    self.r0_enlarge_c9_to_r5 = false;
                    self.vcc.reset();
                    self.vlc.reset();
                    new_frame = true;
                }
            } else if self.in_vertical_adjust {
                // VCC/VLC are held here; only the adjust counter advances,
                // one raster line at a time, until it has topped up the
                // frame by R5 extra lines.
                let extra = u8::from(self.interlace_extra_line);
                let ended = if self.model.has_last_line_state() && self.adjust_counts_to_r5 {
                    // CRTC 0 (ACCC 11.3): there is no C5, C9 counts on to R5, read
                    // live: an R5 lowered below the count makes the 5 bit counter
                    // overflow and go round up to the new value
                    self.vtac = (self.vtac + 1) & 0x1f;
                    self.vtac == (self.regs[5].wrapping_add(extra)) & 0x1f
                } else {
                    self.vtac += 1;
                    self.vtac >= self.regs[5].saturating_add(extra)
                };
                if ended {
                    self.in_vertical_adjust = false;
                    self.interlace_extra_line = false;
                    self.last_line = false;
                    self.vtac = 0;
                    self.vcc.reset();
                    self.vlc.reset();
                    new_frame = true;
                }
            } else if self.model.has_last_line_state() {
                // ACCC 12.2 / 10.3.1: the latched last-line state, or else
                // C9==R9 at this very moment decides everything
                let ivm_now = self.ivm();
                let end_of_row = if self.last_line {
                    true
                } else {
                    let r9 = self.regs[9];
                    let pf = u8::from(self.parity_frame);
                    match (self.ivm_lines, ivm_now) {
                        (false, false) => self.vlc == r9,
                        // the line where R8 goes to 3: C9 against R9 with the parity
                        (false, true) => self.vlc.value() == (r9 | pf),
                        (true, true) => self.c9_is_last(),
                        // back to normal: C9.VMA against R9
                        (true, false) => self.c9_vma() == r9,
                    }
                };
                if self.last_line {
                    self.last_line = false;
                    self.vlc.reset();
                    self.vcc += 1;
                    let extra = self.interlaced() && self.parity_r6;
                    if !(self.additional_armed || self.additional_r5 || extra) {
                        self.vcc.reset();
                        new_frame = true;
                    } else {
                        self.in_vertical_adjust = true;
                        self.adjust_counts_to_r5 = self.additional_r5;
                        self.interlace_extra_line = extra;
                        self.vtac = 0;
                    }
                } else if end_of_row {
                    self.vlc.reset();
                    self.vcc += 1;
                } else {
                    self.vlc += 1;
                }
                self.ivm_lines = ivm_now;
                // parity of the C9 lines of the next row
                self.parity_c9 = if ivm_now && self.regs[9] & 1 != 0 {
                    (self.vcc.value() & 1 != 0) ^ self.parity_frame
                } else {
                    self.parity_frame
                };
            } else {
                // CRTC 1 interlace video mode (ACCC 19.8.2): C9 is incremented by 1 more when R9 is even,
                // the row ends when C9 and R9 match without bit 0, and C9 then restarts from ParityC9
                let asic = self.model.c9_counts_like_asic();
                let ivm_now = (self.model.has_interlace_video() || asic) && self.regs[8] & 3 == 3;
                self.ivm_lines = ivm_now;
                let row_end = if asic {
                    // CRTC 3/4 (ACCC 10.3.4, 19.8.4): an R9 lowered under C9 ends the row too;
                    // in IVM the next row starts on ParityC9, which follows ParityFrame at the
                    // frame start and swaps on each C4 when R9 is odd
                    let r9 = self.regs[9];
                    let end = self.vlc.value() >= r9;
                    if end {
                        if self.vcc == self.regs[4] {
                            // the frame start (below, after any R5 or interlace line, whose
                            // C9 is 0) sets ParityC9
                            self.vlc.reset();
                        } else {
                            if r9 & 1 != 0 {
                                self.parity_c9 = !self.parity_c9;
                            }
                            self.vlc = u8::from(ivm_now && self.parity_c9).into();
                        }
                    } else if ivm_now {
                        self.vlc = (((self.vlc.value() + 2) | u8::from(self.parity_c9)) & 0x1f).into();
                    } else {
                        self.vlc += 1;
                    }
                    end
                } else if ivm_now {
                    let r9 = self.regs[9];
                    let mut c9 = self.vlc.value();
                    if r9 & 1 == 0 {
                        c9 = (c9 + 1) & 0x1f;
                    }
                    let end = c9 & 0x1e == r9 & 0x1e;
                    if end {
                        if r9 & 1 == 0 {
                            self.parity_c9 = !self.parity_c9;
                        }
                        c9 = u8::from(self.parity_c9);
                    } else {
                        c9 = (c9 + 1 + (r9 & 1)) & 0x1f;
                    }
                    self.vlc = c9.into();
                    end
                } else {
                    // compared before the increment: C9 is a 5-bit counter, so with R9=31 the
                    // incremented value (0) never equals R9+1 (32)
                    let end = self.vlc == self.regs[9];
                    self.vlc += 1;
                    end
                };
                // CRTC 2: a "Last Line" sends C9 (and C4) back to 0 whatever R9 became since
                let row_end = row_end || (self.crtc_type() == CrtcType::Crtc2 && self.c2_last_line);

                // This is the end of a char line
                if row_end {
                    if !ivm_now && !asic {
                        self.vlc.reset();
                    }
                    // compared before the increment: C4 is a 7-bit counter, so with R4=127 the
                    // incremented value (0) never equals R4+1 (128)
                    let last_row = if self.crtc_type() == CrtcType::Crtc2 {
                        self.c2_last_line
                    } else {
                        self.vcc == self.regs[4]
                    };
                    self.vcc += 1;

                    if last_row {
                        // ACCC 19.6.2 (CRTC 1): an additional line follows the R5 lines on
                        // the even frames of the interlace modes
                        let extra = self.interlaced() && self.model.frame_parity_toggles() && !self.parity_frame;
                        if self.regs[5] == 0 && !extra {
                            // This is the end of the screen
                            self.vcc.reset();
                            new_frame = true;
                        } else {
                            // Enter vertical total adjust instead of
                            // immediately starting the next frame.
                            if self.model.adjust_keeps_c4() {
                                self.vcc = self.regs[4].into();
                            }
                            self.in_vertical_adjust = true;
                            self.interlace_extra_line = extra;
                            self.vtac = 0;
                        }
                    }
                }
            }

            if self.crtc_type() == CrtcType::Crtc2 {
                self.crtc2_line_start();
            }

            // a new row (or frame, C4 back to 0 with R4=0, or adjustment, C4 kept on R4 on
            // CRTC 3/4) may start a VSYNC again
            if new_frame || self.vcc.value() != vcc_before || (self.in_vertical_adjust && !adjust_before) {
                self.vsync_ended_on_r7 = false;
            }

            // ACCC §19.5.2: ParityFrame takes ParityR6 when a frame starts;
            // ParityR6 anticipates the next frame's parity each time C4
            // reaches R6.
            if self.model.frame_parity_toggles() && new_frame {
                self.parity_frame = !self.parity_frame;
            }
            // ACCC §19.5.5 (CRTC 3/4): ParityC9 = ParityFrame when the frame starts, and in IVM
            // its first line is C9 = ParityC9
            if self.model.c9_counts_like_asic() && new_frame {
                self.parity_c9 = self.parity_frame;
                if self.regs[8] & 3 == 3 {
                    self.vlc = u8::from(self.parity_c9).into();
                }
            }
            if self.model.has_last_line_state() {
                if new_frame {
                    self.parity_frame = self.parity_r6;
                    self.parity_c9 = self.parity_frame;
                }
                if self.vcc == self.regs[6] && (new_frame || self.vcc.value() != vcc_before) {
                    self.parity_r6 = !self.parity_frame;
                }
            }

            // Sampled once per line, at C0=R0 (ACCC §21.3.3); an R6 of 0
            // while the frame is running is not detected by the chip.
            self.vertical_border = self.regs[6] != 0 && self.vcc >= self.regs[6];
        }
        if self.hcc == 2 && self.r0_enlarge_steps_c4 {
            self.r0_enlarge_steps_c4 = false;
            self.vcc += 1;
            if self.vcc == self.regs[6] && self.regs[6] != 0 {
                self.vertical_border = true;
            }
            // ACCC §13.7.2.2: on a last line the additional management stays
            // active whatever R5 is, and C9 goes on counting (no reset) up to R5
            if self.last_line {
                self.r0_enlarge_c9_to_r5 = true;
            }
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
        let by_r1 = if self.model.has_last_line_state() || self.model.r6_zero_is_border() {
            // ACCC §17.4/§17.5: the BORDER R1 is a state set by the C0=R1
            // equality (or R1=0), cleared when a line starts; raising R1
            // afterwards does not bring the display back before the next line
            if self.hcc == 0 {
                self.r1_zero_late = false;
                self.r1_latch = false;
            }
            if self.hcc == self.regs[1] || (self.regs[1] == 0 && !self.r1_zero_late) {
                self.r1_latch = true;
            }
            self.r1_latch
        } else {
            self.hcc >= self.regs[1]
        };
        self.border_history[0] = by_r1 || replaced_by_r0;
        let horizontal_border = match skew {
            DisplaySkew::Delayed(chars) => self.border_history[usize::from(chars)],
            DisplaySkew::Blanked => true,
        };
        let vertical_border = if self.model.r6_zero_is_border() {
            // CRTC 1 (ACCC 18.2.3)
            if self.hcc == 0 && self.vcc == 0 && self.vlc == 0 {
                self.r6_border = false;
            }
            // R6=0 is a live BORDER, not a latched one: C4=R6=0 at the frame start does
            // not latch it, so the display comes back as soon as R6 leaves 0 (Shaker A3,
            // "border state not irreversible", AMSpiriT Lite and CPCSE)
            if self.vcc == self.regs[6] && self.regs[6] != 0 {
                self.r6_border = true;
            }
            self.r6_border || self.regs[6] == 0
        } else if self.model.latches_r6_border() {
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
        } else if self.model.r6_checked_at_line_start() {
            if self.hcc == 0 {
                let first_line = self.vcc == 0
                    && !self.in_vertical_adjust
                    && self.vlc.value() == u8::from(self.ivm_lines && self.parity_c9);
                if first_line {
                    self.r6_border = false;
                }
                if self.vcc == self.regs[6] {
                    self.r6_border = true;
                }
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
        // R3l=2 (CRTC 0): the Gate Array blanks and takes the mode a character before the pulse
        // (AMSpiriT probe on HBL's R3=2 pulses), the pulse itself keeps its place for the
        // interrupt counter
        if self.hsync_is_two_wide() && self.hcc.wrapping_add(1) == self.regs[2] && !self.hsync() {
            self.early_blank_chars = 2;
            self.two_wide_pulse = true;
        }
        if !self.hsync() && self.early_blank_chars == 0 {
            self.two_wide_pulse = false;
        }
        if self.hcc == self.regs[2] && !self.hsync() && !self.hsync_ended_on_char && self.hsync_width().is_some() {
            // R3l=2 written just before the pulse (HBL's panel writes R3=2 on C0=R2-1): too late for the
            // early window, the pulse itself is still not blanked
            if self.hsync_is_two_wide() {
                self.two_wide_pulse = true;
            }
            self.hsc = 0;
            self.on(CrtcPin::HS);
            self.hsync_late_ticks = 0;
            self.hdc = 0; // XXX I am pretty sure I am not reseting it at the right place
        }

        // Request a vsync if needed
        // ACCC §19.7.2: in interlace, on an even frame the VSYNC starts in the
        // middle of the line (C0=R0/2)
        let mid_vsync_wait = self.interlaced() && !self.parity_frame && self.hcc < self.regs[0] / 2;
        // a blocked VSYNC (see r7_match) is released when C4 and R7 differ
        let protected = self.model.has_last_line_state();
        if protected && !self.vsync() && self.vcc != self.regs[7] {
            self.r7_match = false;
        }
        if self.vcc != self.regs[7] {
            self.vsync_ended_on_r7 = false;
        }
        // ACCC §16.4.4 (CRTC 3/4): only on a row's first line (C9=0, or ParityC9 in IVM) at
        // C0=0 (C0=R0/2 for the MID-VSYNC): an R7 set to C4 later in the row does nothing
        let asic_position = !self.model.c9_counts_like_asic()
            || (self.vlc.value() == u8::from(self.ivm_lines && self.parity_c9)
                && self.hcc == if self.interlaced() && !self.parity_frame { self.regs[0] / 2 } else { 0 });
        let vsync_wanted = self.vcc == self.regs[7]
            && !self.vsync()
            && !self.vsync_ended_on_r7
            && !mid_vsync_wait
            && asic_position
            && !(protected && self.r7_match);
        if vsync_wanted && protected && !self.vsync_authorized {
            // ACCC §13.2.2: refused, and this C4=R7 occurrence is spent
            self.vsync_ended_on_r7 = true;
        } else if vsync_wanted {
            self.vsc = 0;
            // ACCC 16.4.1: the line counter of a VSYNC begun mid-line starts at the next C0=0
            self.vsync_skips_first_line_end = protected && self.hcc > 0;
            self.vsync_running = true;
            // the HSYNC window runs up to the character where the HSYNC ends
            let ghost = self.model.ghost_vsync_during_hsync()
                && self.regs[2] != 0
                && (self.hsync() || hsync_before);
            self.set_pin(CrtcPin::VS, !ghost); // Activate Vsync
        }
        if protected {
            if self.hcc == 2 {
                self.vsync_authorized = true;
            } else if self.hcc == 0 {
                self.vsync_authorized = false;
            }
        }

        self.reload_vma_if_needed();
        // CRTC 1 (ACCC 17.5/20.3.2): with R1=0 the condition C0=R1 is met at the first character of
        // the line: on its last scan line VMA' takes VMA as it is at the beginning of this line
        if self.hcc == 0 && self.regs[1] == 0 && self.model.r1_jit_cancels_vma_latch() && self.c9_is_last() {
            self.latch_next_row_address();
        }
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
            self.c9_vma(),
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

/// The CRTC counters of a version 3 snapshot (offsets &A9-&B1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CrtcCounters {
    /// C0
    pub hcc: u8,
    /// C4
    pub vcc: u8,
    /// C9
    pub vlc: u8,
    /// the vertical total adjust counter
    pub vtac: u8,
    /// C3l
    pub hsc: u8,
    /// C3h
    pub vsc: u8,
    pub hsync: bool,
    pub vsync: bool,
    pub in_vertical_adjust: bool
}

impl Crtc {
    /// Restores the CRTC from a snapshot: the registers as stored (no write side
    /// effect: they are not written in the middle of a frame), the selected
    /// register and, from a version 3 snapshot, the counters. Without counters
    /// the CRTC restarts at the top of a frame.
    pub fn restore(&mut self, regs: &[u8; 18], selected: u8, counters: Option<CrtcCounters>) {
        self.reset();
        for (r, &v) in regs.iter().enumerate() {
            self.regs[r] = v & self.model.write_mask(r as u8);
        }
        self.ar = selected & 0b11111;
        let c = counters.unwrap_or_default();
        self.hcc = c.hcc;
        self.vcc = (c.vcc & 0x7f).into();
        self.vlc = (c.vlc & 0x1f).into();
        self.vtac = c.vtac;
        self.hsc = c.hsc & 0x0f;
        self.vsc = c.vsc & 0x0f;
        self.in_vertical_adjust = c.in_vertical_adjust;
        self.adjust_counts_to_r5 = c.in_vertical_adjust;
        self.last_line = false;
        self.vsync_running = c.vsync;
        self.set_pin(CrtcPin::VS, c.vsync);
        self.set_pin(CrtcPin::HS, c.hsync);
        // VMA as the standard frame has it: the row starts R1 characters after the previous one
        let row = if c.in_vertical_adjust { 0 } else { u16::from(c.vcc & 0x7f) };
        let row_start = self.base_address().wrapping_add(row * u16::from(self.regs[1])) & 0x3fff;
        self.crtc_vma = row_start.wrapping_add(u16::from(c.hcc)) & 0x3fff;
        self.crtc_vma_prime = if self.c9_is_last() && c.hcc > self.regs[1] {
            row_start.wrapping_add(u16::from(self.regs[1])) & 0x3fff
        } else {
            row_start
        };
    }

    /// The registers as stored, as a snapshot stores them.
    pub fn registers(&self) -> [u8; 18] {
        let mut regs = [0; 18];
        regs.copy_from_slice(&self.regs[..18]);
        regs
    }

    /// The counters, as a version 3 snapshot stores them.
    pub fn counters(&self) -> CrtcCounters {
        CrtcCounters {
            hcc: self.hcc,
            vcc: self.vcc.value(),
            vlc: self.vlc.value(),
            vtac: self.vtac,
            hsc: self.hsc,
            vsc: self.vsc,
            hsync: self.hsync(),
            vsync: self.vsync(),
            in_vertical_adjust: self.in_vertical_adjust
        }
    }

    /// Check if the CRTC as generated a VSYNC
    /// (A ghost VSYNC, see [`CrtcModel::ghost_vsync_during_hsync`], runs without the VS pin.)
    pub fn vsync(&self) -> bool {
        self.vsync_running
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
        // nothing of the previous run survives a reset
        self.vsc = 0;
        self.hsc = 0;
        self.hdc = 0;
        self.additional_armed = false;
        self.additional_r5 = false;
        self.r0_enlarge_steps_c4 = false;
        self.r0_enlarge_c9_to_r5 = false;
        self.adjust_counts_to_r5 = false;
        self.r7_match = false;
        self.early_blank_chars = 0;
        self.two_wide_pulse = false;
        self.vsync_skips_first_line_end = false;
        self.vsync_ended_on_r7 = false;
        self.vsync_running = false;
        self.vsync_authorized = true;
        self.r1_latch = false;
        self.r1_zero_late = false;
        self.hsync_late_ticks = 0;
        self.vertical_border = false;
        self.border_history = [false; 3];
        self.r6_border = false;
        self.r0_zero_wraps = 0;
        self.parity_frame = false;
        self.parity_r6 = false;
        self.parity_c9 = false;
        self.ivm_lines = false;
        self.interlace_extra_line = false;
        self.rfd_active = false;
        self.vma_latch_pending = None;
        self.r0_zero_c4_pending = false;
        self.r6_zero_seen = false;

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

    fn crtc0() -> Crtc {
        Crtc::from(CrtcType::Crtc0)
    }

    fn step_until_hcc(crtc: &mut Crtc, hcc: u8) {
        for _ in 0..100_000 {
            crtc.update_memory_address();
            if crtc.hcc == hcc {
                return;
            }
        }
        panic!("C0 never reached {hcc}");
    }

    /// ACCC §17.4/§17.5: once the BORDER R1 has started on a line (here an
    /// R1=0 on C0=0), raising R1 does not bring the display back before the
    /// next line.
    #[test]
    fn test_crtc0_border_r1_is_latched_for_the_line() {
        let mut crtc = crtc0();
        step_until_hcc(&mut crtc, 0);
        crtc.do_write_register(1, 0);
        assert!(!crtc.get_pin(CrtcPin::DE));
        step_until_hcc(&mut crtc, 3);
        crtc.do_write_register(1, 40);
        step_until_hcc(&mut crtc, 8);
        assert!(!crtc.get_pin(CrtcPin::DE), "the border must stay until the next line");
        step_until_hcc(&mut crtc, 2);
        assert!(crtc.get_pin(CrtcPin::DE), "the next line displays again");
    }

    /// ACCC §13.7.2.1: R0 enlarged out of 1 on C0=1 while C9=R9 overflows C4
    /// on C0=2, even if C4<>R4.
    #[test]
    fn test_crtc0_r0_enlarged_from_one_steps_c4() {
        let mut crtc = crtc0();
        crtc.do_write_register(9, 0);
        crtc.do_write_register(4, 100);
        crtc.do_write_register(0, 1);
        step_until_hcc(&mut crtc, 1);
        while crtc.vlc.value() != 0 {
            step_until_hcc(&mut crtc, 1);
        }
        let before = crtc.vcc.value();
        crtc.do_write_register(0, 63);
        crtc.update_memory_address();
        assert_eq!(crtc.hcc, 2);
        assert_eq!(crtc.vcc.value(), (before + 1) & 0x7f);
    }

    /// ACCC §13.2.6: with R0=0 the first C0=0 only compares C9 (C9<>R9: it
    /// moves once), then C9 and C4 are frozen.
    #[test]
    fn test_crtc0_r0_zero_freezes_the_counters() {
        let mut crtc = crtc0();
        step_until_hcc(&mut crtc, 0);
        for _ in 0..3 * 64 {
            crtc.update_memory_address();
        }
        step_until_hcc(&mut crtc, 0);
        let (c4, c9) = (crtc.vcc.value(), crtc.vlc.value());
        assert!(c9 < 6, "need room to count");
        crtc.do_write_register(0, 0);
        for _ in 0..50 {
            crtc.update_memory_address();
        }
        assert_eq!(crtc.vcc.value(), c4);
        assert_eq!(crtc.vlc.value(), c9 + 1);
    }

    /// One line of 64 characters
    fn run_line(crtc: &mut Crtc) {
        let ends = crtc.line_ends;
        while crtc.line_ends == ends {
            crtc.update_memory_address();
        }
    }

    /// The VSYNC line counter is 4 bits: an R3 lowered below the count makes it
    /// go round.
    #[test]
    fn test_crtc0_vsync_counter_goes_round_when_r3_is_lowered() {
        let mut crtc = crtc0();
        while !crtc.vsync() {
            crtc.update_memory_address();
        }
        for _ in 0..4 {
            run_line(&mut crtc);
        }
        crtc.do_write_register(3, 0x20 | 0x0e);
        let mut more = 0;
        while crtc.vsync() {
            run_line(&mut crtc);
            more += 1;
        }
        assert_eq!(more, 16 - 4 + 2);
    }

    /// ACCC §11.2.2: on CRTC 0 C9 itself counts the additional lines, and goes
    /// out on RA.
    #[test]
    fn test_crtc0_c9_counts_the_additional_lines() {
        let mut crtc = crtc0();
        crtc.do_write_register(5, 3);
        let mut seen = Vec::new();
        for _ in 0..400 {
            run_line(&mut crtc);
            if crtc.vcc.value() == crtc.regs[4] + 1 {
                seen.push(crtc.c9());
            }
        }
        assert!(seen.starts_with(&[0, 1, 2]), "{seen:?}");
    }

    /// ACCC §16.4.1: a VSYNC triggered by R7 in the middle of a line does not
    /// count that line.
    #[test]
    fn test_crtc0_vsync_started_mid_line_lasts_one_line_more() {
        let mut crtc = crtc0();
        crtc.do_write_register(7, 99);
        while crtc.vcc.value() != 31 {
            run_line(&mut crtc);
        }
        step_until_hcc(&mut crtc, 20);
        crtc.do_write_register(7, 31);
        crtc.update_memory_address();
        assert!(crtc.vsync());
        let mut lines = 0;
        while crtc.vsync() {
            run_line(&mut crtc);
            lines += 1;
        }
        assert_eq!(lines, 1 + 8);
    }

    /// Steps character by character, with a bound so a missing VSYNC fails instead of hanging.
    fn step_while(crtc: &mut Crtc, vsync: bool) {
        for _ in 0..1_000_000 {
            if crtc.vsync() != vsync {
                return;
            }
            crtc.update_memory_address();
        }
        panic!("VSYNC stuck {}", if vsync { "high" } else { "low" });
    }

    /// (VSYNC length, VSYNC period) in scan lines, after a few frames to settle.
    fn vsync_and_frame_lines(crtc: &mut Crtc) -> (u64, u64) {
        for _ in 0..3 {
            step_while(crtc, false);
            step_while(crtc, true);
        }
        step_while(crtc, false);
        let start = crtc.line_ends;
        step_while(crtc, true);
        let high = crtc.line_ends - start;
        step_while(crtc, false);
        (high, crtc.line_ends - start)
    }

    fn crtc_with(kind: CrtcType, regs: &[(u8, u8)]) -> Crtc {
        let mut crtc = Crtc::from(kind);
        for &(r, v) in regs {
            crtc.do_write_register(r, v);
        }
        crtc
    }

    const ALL_TYPES: [CrtcType; 5] = [CrtcType::Crtc0, CrtcType::Crtc1, CrtcType::Crtc2, CrtcType::Crtc3, CrtcType::Crtc4];

    /// C9 is a 5-bit counter: rows of 32 lines (R9=31) still end, on every CRTC
    /// (AMSpiriT Lite and CPCSE: 8 rows of 32 lines make a 256-line frame).
    #[test]
    fn test_rows_of_32_lines() {
        for kind in ALL_TYPES {
            let mut crtc = crtc_with(kind, &[(9, 31), (4, 7), (7, 2), (6, 6)]);
            let width = u64::from(crtc.vsync_height());
            assert_eq!(vsync_and_frame_lines(&mut crtc), (width, 8 * 32), "{kind:?}");
        }
    }

    /// C4 is a 7-bit counter: with R4=127 the frame still ends after row 127 and
    /// takes its R5 lines (AMSpiriT Lite and CPCSE: 128 + 4 lines).
    #[test]
    fn test_r4_127_keeps_the_adjust_lines() {
        for kind in ALL_TYPES {
            let mut crtc = crtc_with(kind, &[(9, 0), (4, 127), (5, 4), (7, 100), (6, 50)]);
            assert_eq!(vsync_and_frame_lines(&mut crtc).1, 128 + 4, "{kind:?}");
        }
    }

    /// A VSYNC that ends while C4 still equals R7 does not start again on the
    /// same row (AMSpiriT Lite and CPCSE: R3=&1E gives 1 line, not the whole row).
    #[test]
    fn test_vsync_does_not_restart_on_the_same_row() {
        for kind in [CrtcType::Crtc0, CrtcType::Crtc3, CrtcType::Crtc4] {
            for (r3, lines) in [(0x1e, 1), (0x4e, 4)] {
                let mut crtc = crtc_with(kind, &[(3, r3)]);
                assert_eq!(vsync_and_frame_lines(&mut crtc), (lines, 39 * 8), "{kind:?} R3={r3:#x}");
            }
        }
        // rows longer than the 16 lines of a CRTC 1/2 VSYNC
        for kind in ALL_TYPES {
            let mut crtc = crtc_with(kind, &[(9, 31), (4, 7), (7, 2), (6, 6)]);
            let width = u64::from(crtc.vsync_height());
            assert_eq!(vsync_and_frame_lines(&mut crtc).0, width, "{kind:?}");
        }
    }

    /// One-row frames (R4=0): C4 stays 0, yet each frame has its VSYNC. On CRTC 3/4
    /// C4 also stays 0 for the R5 lines (ACCC §18.2.4), where a VSYNC starts again and
    /// runs on into the next frame's: 16 lines (AMSpiriT Lite and CPCSE).
    #[test]
    fn test_vsync_every_frame_of_a_single_row() {
        for kind in ALL_TYPES {
            let mut crtc = crtc_with(kind, &[(4, 0), (9, 31), (5, 8), (7, 0), (6, 0)]);
            let width = u64::from(crtc.vsync_height());
            let high = if crtc.model.adjust_keeps_c4() { 8 + width } else { width };
            assert_eq!(vsync_and_frame_lines(&mut crtc), (high, 32 + 8), "{kind:?}");
        }
    }

    /// A reset forgets the transient state of the previous run.
    #[test]
    fn test_reset_clears_the_vsync_state() {
        let mut crtc = crtc_with(CrtcType::Crtc0, &[(3, 0x1e)]);
        step_while(&mut crtc, false);
        step_while(&mut crtc, true);
        crtc.reset();
        assert!(!crtc.vsync_ended_on_r7 && !crtc.r7_match && !crtc.vsync_skips_first_line_end);
    }

    /// ACCC §13.2.2: a CRTC 0 line must reach C0=2 for the next C0=0 to start a
    /// VSYNC, so with R0=1 there is none (AMSpiriT Lite and CPCSE); CRTC 1 has it.
    #[test]
    fn test_crtc0_no_vsync_with_r0_1() {
        let regs = [(0, 1), (1, 1), (2, 1), (3, 0x81)];
        let mut crtc = crtc_with(CrtcType::Crtc0, &regs);
        for _ in 0..200_000 {
            crtc.update_memory_address();
            assert!(!crtc.vsync());
        }
        let mut crtc = crtc_with(CrtcType::Crtc1, &regs);
        step_while(&mut crtc, false);
    }

    /// ACCC §16.4.3: on CRTC 2 a VSYNC condition met during the HSYNC (R0=31, the
    /// HSYNC of R2=24 runs past C0=0) is a GHOST VSYNC: it runs, the pin stays low
    /// (CPCSE). With R2=0 it is a real one.
    #[test]
    fn test_crtc2_ghost_vsync_during_hsync() {
        let mut crtc = crtc_with(CrtcType::Crtc2, &[(0, 31), (1, 20), (2, 24)]);
        let (ran, _) = vsync_and_frame_lines(&mut crtc);
        assert_eq!(ran, 16);
        for _ in 0..100_000 {
            crtc.update_memory_address();
            assert!(!crtc.get_pin(CrtcPin::VS));
        }
        let mut crtc = crtc_with(CrtcType::Crtc2, &[(0, 31), (1, 20), (2, 0)]);
        step_while(&mut crtc, false);
        assert!(crtc.get_pin(CrtcPin::VS));
        // the other chips have no ghost
        let mut crtc = crtc_with(CrtcType::Crtc0, &[(0, 31), (1, 20), (2, 24)]);
        step_while(&mut crtc, false);
        assert!(crtc.get_pin(CrtcPin::VS));
    }

    /// Interlace sync (R8=1, and R8=3 on CRTC 2): VSYNC every 312.5 lines, the
    /// half line alternating (AMSpiriT Lite and CPCSE), on CRTC 1 to 4.
    #[test]
    fn test_interlace_sync_on_crtc_1_to_4() {
        for kind in [CrtcType::Crtc1, CrtcType::Crtc2, CrtcType::Crtc3, CrtcType::Crtc4] {
            for r8 in [1, 3] {
                if r8 == 3 && kind != CrtcType::Crtc2 {
                    continue; // interlace video: CRTC 1 has its own, CRTC 3/4 not emulated
                }
                let mut crtc = crtc_with(kind, &[(8, r8)]);
                vsync_and_frame_lines(&mut crtc);
                // characters between VSYNC starts: 312.5 lines of 64 on average
                let mut chars = Vec::new();
                for _ in 0..4 {
                    let mut n = 0u64;
                    while crtc.vsync() {
                        crtc.update_memory_address();
                        n += 1;
                    }
                    while !crtc.vsync() {
                        crtc.update_memory_address();
                        n += 1;
                    }
                    chars.push(n);
                }
                assert_eq!(chars.iter().sum::<u64>(), 4 * 20000, "{kind:?} R8={r8}: {chars:?}");
            }
        }
    }

    /// CRTC 3/4 end the line as soon as R0 is lowered below C0; CRTC 0 lets C0 count on to
    /// 255 (AMSpiriT Lite and CPCSE, Shaker A5-A7: R0=3 ruptures).
    #[test]
    fn test_asic_line_ends_when_r0_is_lowered_below_c0() {
        for (kind, at_once) in [(CrtcType::Crtc4, true), (CrtcType::Crtc3, true), (CrtcType::Crtc0, false)] {
            let mut crtc = crtc_with(kind, &[]);
            while crtc.hcc != 40 {
                crtc.update_memory_address();
            }
            let ends = crtc.line_ends;
            crtc.do_write_register(0, 10);
            crtc.update_memory_address();
            assert_eq!(crtc.line_ends == ends + 1, at_once, "{kind:?}");
            if !at_once {
                assert_eq!(crtc.hcc, 41);
            }
        }
    }

    /// CRTC 2 (ACCC §13.4): the frame starts at VMA', which takes R12/R13 at C0=R1 on the
    /// frame's last line. R12/R13 written after that only count from the frame after
    /// (AMSpiriT Lite and CPCSE: Shaker AO, R1>R0, starts every frame from the stale VMA').
    /// CRTC 0 takes R12/R13 at the frame's first line.
    #[test]
    fn test_crtc2_frame_starts_at_vma_prime() {
        for (kind, delayed) in [(CrtcType::Crtc2, true), (CrtcType::Crtc0, false)] {
            let mut crtc = crtc_with(kind, &[]);
            // the last line of the frame, past C0=R1
            while !(crtc.vcc == crtc.regs[4] && crtc.vlc == crtc.regs[9] && crtc.hcc == crtc.regs[1] + 2) {
                crtc.update_memory_address();
            }
            crtc.do_write_register(12, 0x10);
            while !(crtc.vcc == 0 && crtc.vlc == 0 && crtc.hcc == 1) {
                crtc.update_memory_address();
            }
            let expected = if delayed { 0x3000 } else { 0x1000 };
            assert_eq!(crtc.crtc_vma & 0x3fc0, expected, "{kind:?}");
        }
    }

    /// CRTC 1: R6=0 is a live BORDER. Set on the last line, it is not latched by the
    /// next frame's C4=R6=0: R6 back to 25 brings the display back (Shaker A3,
    /// AMSpiriT Lite and CPCSE). Written on the first line of a frame it is final.
    #[test]
    fn test_crtc1_r6_zero_border_is_not_latched() {
        let mut crtc = crtc_with(CrtcType::Crtc1, &[]);
        while !(crtc.vcc == crtc.regs[4] && crtc.vlc == crtc.regs[9]) {
            crtc.update_memory_address();
        }
        crtc.do_write_register(6, 0);
        while crtc.vcc != 1 {
            crtc.update_memory_address();
            assert!(!crtc.get_pin(CrtcPin::DE) || crtc.hcc >= crtc.regs[1] || crtc.vcc != 0, "R6=0: border");
        }
        crtc.do_write_register(6, 25);
        while crtc.hcc != 5 {
            crtc.update_memory_address();
        }
        assert!(crtc.get_pin(CrtcPin::DE), "R6 back to 25: display");

        // R6=0 written on the first line of the frame: BORDER until the next frame
        while !(crtc.vcc == 0 && crtc.vlc == 0 && crtc.hcc == 5) {
            crtc.update_memory_address();
        }
        crtc.do_write_register(6, 0);
        while crtc.vcc != 1 {
            crtc.update_memory_address();
        }
        crtc.do_write_register(6, 25);
        while crtc.hcc != 5 {
            crtc.update_memory_address();
        }
        assert!(!crtc.get_pin(CrtcPin::DE), "R6=0 on C4=C9=0: final border");
    }
}
