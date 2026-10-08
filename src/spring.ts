/** Damped harmonic spring, integrated in fixed sub-steps so it is frame-rate independent. */
export class Spring {
  velocity = 0;

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
    while (remaining > 0) {
      const s = Math.min(h, remaining);
      const accel = (-this.stiffness * (this.value - this.target) - this.damping * this.velocity) / this.mass;
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

  configure(stiffness: number, damping: number, mass: number): void {
    this.stiffness = stiffness;
    this.damping = damping;
    this.mass = Math.max(0.01, mass);
  }
}
