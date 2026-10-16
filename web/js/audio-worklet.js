// The sound of bemu (see bemu.js): stereo samples, interleaved, posted by the emulator a frame at
// a time and played from a ring buffer; silence when it runs dry.

class BemuAudio extends AudioWorkletProcessor {
    constructor() {
        super();
        this.ring = new Float32Array(2 * Math.ceil(sampleRate / 2)); // half a second
        this.read = 0;
        this.size = 0;
        this.port.onmessage = (event) => this.push(event.data);
    }

    push(samples) {
        const ring = this.ring;
        for (let i = 0; i < samples.length; i++) {
            if (this.size === ring.length) {
                // full: the oldest sample goes
                this.read = (this.read + 1) % ring.length;
                this.size--;
            }
            ring[(this.read + this.size) % ring.length] = samples[i];
            this.size++;
        }
    }

    process(_inputs, outputs) {
        const [left, right] = outputs[0];
        const ring = this.ring;
        for (let i = 0; i < left.length; i++) {
            if (this.size >= 2) {
                left[i] = ring[this.read];
                right[i] = ring[(this.read + 1) % ring.length];
                this.read = (this.read + 2) % ring.length;
                this.size -= 2;
            } else {
                left[i] = 0;
                right[i] = 0;
            }
        }
        return true;
    }
}

registerProcessor('bemu-audio', BemuAudio);
