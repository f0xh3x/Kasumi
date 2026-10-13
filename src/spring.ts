/** Damped harmonic spring, integrated in fixed sub-steps so it is frame-rate independent. */
export class Spring {
  /**
   * [animation] bounce. Off: every spring is at least critically damped, so nothing
   * overshoots where it's heading (a 2% overshoot read as a bounce or a glitch on
   * every open, close and morph); on: as damped as configured.
   */
  static bounce = false;
  velocity = 0;
  /** Speed while heading to 1 (opening) and to 0 (closing): 2 = twice as fast, same bounce. */
  open = 1;
  close = 1;

  constructor(
    public value: number,
    public target: number,
    public stiffness: number,
    public damping: number,
    public mass = 1,
  ) {}

  step(dt: number): void {
    const h = 1 / 240;
    let remaining = dt;
    // Scaling stiffness by f² and damping by f runs the same motion f times faster.
    const f = this.target >= 0.5 ? this.open : this.close;
    const k = this.stiffness * f * f;
    const c = Spring.bounce ? this.damping * f : Math.max(this.damping * f, 2 * Math.sqrt(k * this.mass));
    while (remaining > 0) {
      const s = Math.min(h, remaining);
      const accel = (-k * (this.value - this.target) - c * this.velocity) / this.mass;
      this.velocity += accel * s;
      this.value += this.velocity * s;
      remaining -= s;
    }
  }

  get settled(): boolean {
    return Math.abs(this.value - this.target) < 5e-4 && Math.abs(this.velocity) < 5e-3;
  }

  snap(): void {
    this.value = this.target;
    this.velocity = 0;
  }

  /** Opening / closing speed ([animation] open_speed / close_speed). */
  speed(open: number, close: number): void {
    const ok = (v: number) => (Number.isFinite(v) && v > 0 ? Math.min(4, Math.max(0.25, v)) : 1);
    this.open = ok(open);
    this.close = ok(close);
  }

  configure(stiffness: number, damping: number, mass: number): void {
    this.stiffness = stiffness;
    this.damping = damping;
    this.mass = Math.max(0.01, mass);
  }
}
