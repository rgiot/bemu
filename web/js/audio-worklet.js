// The sound of bemu (see bemu.js): stereo samples, interleaved, posted by the emulator a frame at
// a time and played from a ring buffer.
//
// - Playback starts once `target` frames are queued, and again after an underrun, so a late
//   frame does not turn into a stream of clicks.
// - The emulator is paced by the display, whose clock is not the sound card's: the playback rate
//   follows the fill level (within RATE_RANGE) instead of letting the buffer run dry or overflow.
// - An underrun fades out and an (re)start fades in: no step in the signal.
// - Whole stereo frames only: the channels never swap.
//
// Messages in: a Float32Array of samples, or { target } (seconds). Out, about every 20 ms:
// { fill, time, playing, underruns }, the queued frames and the context's time of the report.

const RATE_RANGE = 0.01;
const RATE_SMOOTHING = 0.02;
const FADE_FRAMES = 128;
const REPORT_BLOCKS = 8;

class BemuAudio extends AudioWorkletProcessor {
    constructor(options) {
        super();
        const wanted = options?.processorOptions?.target ?? 0.06;
        this.capacity = Math.ceil(sampleRate); // one second
        this.left = new Float32Array(this.capacity);
        this.right = new Float32Array(this.capacity);
        this.target = Math.max(1, Math.round(wanted * sampleRate));
        this.written = 0; // frames received, ever
        this.position = 0; // frames played, ever (fractional: the rate follows the fill)
        this.rate = 1;
        this.playing = false;
        this.fade = 0; // frames since the (re)start, up to FADE_FRAMES
        this.lastLeft = 0;
        this.lastRight = 0;
        this.underruns = 0;
        this.blocks = 0;
        this.port.onmessage = (event) => this.receive(event.data);
    }

    receive(data) {
        if (ArrayBuffer.isView(data)) this.push(data);
        else if (data && typeof data.target === 'number') this.target = Math.max(1, Math.round(data.target * sampleRate));
    }

    push(samples) {
        let frames = samples.length >> 1;
        let from = 0;
        if (frames > this.capacity) { // more than the buffer holds: the newest
            from = frames - this.capacity;
            frames = this.capacity;
        }
        const excess = this.written + frames - this.position - this.capacity;
        if (excess > 0) this.position += Math.ceil(excess); // full: the oldest frames go
        for (let i = 0; i < frames; i++) {
            const slot = (this.written + i) % this.capacity;
            this.left[slot] = samples[2 * (from + i)];
            this.right[slot] = samples[2 * (from + i) + 1];
        }
        this.written += frames;
    }

    process(_inputs, outputs) {
        const [left, right] = outputs[0];
        const length = left.length;
        let i = 0;

        if (!this.playing && this.written - this.position >= this.target) {
            this.playing = true;
            this.fade = 0;
        }

        if (this.playing) {
            const fill = this.written - this.position;
            const error = Math.max(-1, Math.min(1, (fill - this.target) / this.target));
            this.rate += (1 + RATE_RANGE * error - this.rate) * RATE_SMOOTHING;
            const capacity = this.capacity;
            for (; i < length; i++) {
                const index = Math.floor(this.position);
                if (this.written - index < 2) { // not even two frames to interpolate
                    this.playing = false;
                    this.underruns++;
                    break;
                }
                const fraction = this.position - index;
                const a = index % capacity;
                const b = (index + 1) % capacity;
                const gain = this.fade < FADE_FRAMES ? ++this.fade / FADE_FRAMES : 1;
                this.lastLeft = (this.left[a] + (this.left[b] - this.left[a]) * fraction) * gain;
                this.lastRight = (this.right[a] + (this.right[b] - this.right[a]) * fraction) * gain;
                left[i] = this.lastLeft;
                right[i] = this.lastRight;
                this.position += this.rate;
            }
        }

        // Silent: the last sample fades out over the rest of the block, then nothing.
        const remaining = length - i;
        for (let k = 0; k < remaining; k++, i++) {
            const gain = 1 - (k + 1) / Math.min(remaining, FADE_FRAMES);
            left[i] = this.lastLeft * gain;
            right[i] = this.lastRight * gain;
        }
        if (remaining > 0) {
            this.lastLeft = 0;
            this.lastRight = 0;
        }

        if (++this.blocks >= REPORT_BLOCKS) {
            this.blocks = 0;
            this.port.postMessage({
                fill: Math.max(0, this.written - this.position),
                time: currentTime,
                playing: this.playing,
                underruns: this.underruns
            });
        }
        return true;
    }
}

registerProcessor('bemu-audio', BemuAudio);
