/// Pegas teredam untuk animasi ukuran notch. Sedikit overshoot supaya terasa hidup.
pub struct Spring {
    pub value: f32,
    target: f32,
    vel: f32,
}

const STIFFNESS: f32 = 340.0;
const DAMPING: f32 = 30.0;

impl Spring {
    pub fn new(v: f32) -> Self {
        Self { value: v, target: v, vel: 0.0 }
    }

    pub fn set(&mut self, t: f32) {
        self.target = t;
    }

    /// Maju `dt` detik. Mengembalikan `true` selama masih bergerak.
    pub fn step(&mut self, dt: f32) -> bool {
        // Langkah kecil tetap supaya stabil walau timer Windows tidak presisi.
        let mut left = dt.min(0.1);
        while left > 0.0 {
            let h = left.min(1.0 / 240.0);
            let force = STIFFNESS * (self.target - self.value) - DAMPING * self.vel;
            self.vel += force * h;
            self.value += self.vel * h;
            left -= h;
        }
        if (self.target - self.value).abs() < 0.25 && self.vel.abs() < 2.0 {
            self.value = self.target;
            self.vel = 0.0;
            return false;
        }
        true
    }
}
