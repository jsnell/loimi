// Raytrace: a small recursive ray tracer in the spirit of the V8/Octane
// "raytrace" benchmark (Adam Burmister's Flog.RayTracer): spheres and a
// checkered plane, shadows, reflections, Phong specular highlights.
// Object-heavy float math: every vector operation allocates.
// Only + - * / and Math.sqrt are used on floats, so the result is exact.
const SIZE = 300;       // image is SIZE x SIZE pixels
const MAX_DEPTH = 3;    // reflection depth

class Vec {
  constructor(x, y, z) {
    this.x = x;
    this.y = y;
    this.z = z;
  }
  add(o) { return new Vec(this.x + o.x, this.y + o.y, this.z + o.z); }
  sub(o) { return new Vec(this.x - o.x, this.y - o.y, this.z - o.z); }
  scale(k) { return new Vec(this.x * k, this.y * k, this.z * k); }
  dot(o) { return this.x * o.x + this.y * o.y + this.z * o.z; }
  cross(o) {
    return new Vec(this.y * o.z - this.z * o.y,
                   this.z * o.x - this.x * o.z,
                   this.x * o.y - this.y * o.x);
  }
  length() { return Math.sqrt(this.dot(this)); }
  normalize() { return this.scale(1 / this.length()); }
}

class Color {
  constructor(r, g, b) {
    this.r = r;
    this.g = g;
    this.b = b;
  }
  add(o) { return new Color(this.r + o.r, this.g + o.g, this.b + o.b); }
  mul(o) { return new Color(this.r * o.r, this.g * o.g, this.b * o.b); }
  scale(k) { return new Color(this.r * k, this.g * k, this.b * k); }
  clamp() {
    return new Color(Math.min(1, Math.max(0, this.r)),
                     Math.min(1, Math.max(0, this.g)),
                     Math.min(1, Math.max(0, this.b)));
  }
}

const BLACK = new Color(0, 0, 0);

class Ray {
  constructor(origin, dir) {
    this.origin = origin;
    this.dir = dir;
  }
}

class Hit {
  constructor(shape, dist, pos, normal) {
    this.shape = shape;
    this.dist = dist;
    this.pos = pos;
    this.normal = normal;
  }
}

class Material {
  constructor(reflection, gloss) {
    this.reflection = reflection;
    this.gloss = gloss;
  }
  colorAt(pos) { throw new Error("abstract"); }
}

class Solid extends Material {
  constructor(color, reflection, gloss) {
    super(reflection, gloss);
    this.color = color;
  }
  colorAt(pos) { return this.color; }
}

class Checker extends Material {
  constructor(c1, c2, density, reflection, gloss) {
    super(reflection, gloss);
    this.c1 = c1;
    this.c2 = c2;
    this.density = density;
  }
  colorAt(pos) {
    const t = Math.floor(pos.x * this.density) + Math.floor(pos.z * this.density);
    return (t & 1) === 0 ? this.c1 : this.c2;
  }
}

class Shape {
  constructor(material) {
    this.material = material;
  }
}

class Sphere extends Shape {
  constructor(center, radius, material) {
    super(material);
    this.center = center;
    this.radius2 = radius * radius;
  }
  intersect(ray) {
    const oc = ray.origin.sub(this.center);
    const b = oc.dot(ray.dir);
    const c = oc.dot(oc) - this.radius2;
    const disc = b * b - c;
    if (disc <= 0) return null;
    const d = -b - Math.sqrt(disc);
    if (d <= 1e-6) return null;
    const pos = ray.origin.add(ray.dir.scale(d));
    return new Hit(this, d, pos, pos.sub(this.center).normalize());
  }
}

class Plane extends Shape {
  constructor(normal, offset, material) {
    super(material);
    this.normal = normal;
    this.offset = offset;
  }
  intersect(ray) {
    const vd = this.normal.dot(ray.dir);
    if (vd >= 0) return null;
    const t = -(this.normal.dot(ray.origin) + this.offset) / vd;
    if (t <= 1e-6) return null;
    const pos = ray.origin.add(ray.dir.scale(t));
    return new Hit(this, t, pos, this.normal);
  }
}

class Light {
  constructor(pos, color) {
    this.pos = pos;
    this.color = color;
  }
}

class Camera {
  constructor(pos, lookAt, up) {
    this.pos = pos;
    this.forward = lookAt.sub(pos).normalize();
    this.right = this.forward.cross(up).normalize();
    this.up = this.right.cross(this.forward).normalize();
  }
  rayFor(vx, vy) {
    const dir = this.forward.add(this.right.scale(vx)).add(this.up.scale(vy));
    return new Ray(this.pos, dir.normalize());
  }
}

class Scene {
  constructor() {
    this.shapes = [];
    this.lights = [];
    this.camera = null;
    this.background = new Color(0.1, 0.1, 0.15);
  }
  nearest(ray) {
    let best = null;
    for (const s of this.shapes) {
      const h = s.intersect(ray);
      if (h !== null && (best === null || h.dist < best.dist)) best = h;
    }
    return best;
  }
  shadowed(pos, toLight, dist) {
    const ray = new Ray(pos, toLight);
    for (const s of this.shapes) {
      const h = s.intersect(ray);
      if (h !== null && h.dist < dist) return true;
    }
    return false;
  }
  trace(ray, depth) {
    const hit = this.nearest(ray);
    if (hit === null) return this.background;
    const mat = hit.shape.material;
    const base = mat.colorAt(hit.pos);
    let color = base.scale(0.1);
    for (const light of this.lights) {
      const lv = light.pos.sub(hit.pos);
      const ldist = lv.length();
      const ldir = lv.scale(1 / ldist);
      if (this.shadowed(hit.pos, ldir, ldist)) continue;
      const diff = hit.normal.dot(ldir);
      if (diff > 0) color = color.add(base.mul(light.color).scale(diff));
      if (mat.gloss > 0) {
        const h = ldir.sub(ray.dir).normalize();
        let spec = hit.normal.dot(h);
        if (spec > 0) {
          for (let i = 0; i < mat.gloss; i++) spec = spec * spec;
          color = color.add(light.color.scale(spec));
        }
      }
    }
    if (depth < MAX_DEPTH && mat.reflection > 0) {
      const d = ray.dir;
      const r = d.sub(hit.normal.scale(2 * d.dot(hit.normal)));
      const refl = this.trace(new Ray(hit.pos, r), depth + 1);
      color = color.add(refl.scale(mat.reflection));
    }
    return color;
  }
}

function buildScene() {
  const scene = new Scene();
  scene.camera = new Camera(new Vec(0, 2, -7), new Vec(0, 0.8, 0), new Vec(0, 1, 0));
  scene.shapes.push(new Plane(new Vec(0, 1, 0), 0,
    new Checker(new Color(0.9, 0.9, 0.9), new Color(0.1, 0.1, 0.1), 1, 0.3, 0)));
  scene.shapes.push(new Sphere(new Vec(-1.2, 1, 0), 1,
    new Solid(new Color(0.8, 0.2, 0.2), 0.4, 4)));
  scene.shapes.push(new Sphere(new Vec(1.3, 0.8, -0.5), 0.8,
    new Solid(new Color(0.2, 0.3, 0.9), 0.5, 5)));
  scene.shapes.push(new Sphere(new Vec(0.2, 0.4, -2), 0.4,
    new Solid(new Color(0.2, 0.8, 0.3), 0.2, 3)));
  scene.shapes.push(new Sphere(new Vec(0, 3.2, 1.5), 0.9,
    new Solid(new Color(0.9, 0.8, 0.2), 0.6, 6)));
  scene.lights.push(new Light(new Vec(-5, 6, -4), new Color(0.8, 0.8, 0.8)));
  scene.lights.push(new Light(new Vec(4, 4, -6), new Color(0.4, 0.4, 0.5)));
  return scene;
}

function render(scene, size) {
  let sum = 0;
  let hash = 0;
  const rowStr = [];
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const vx = (x - size / 2) / size;
      const vy = (size / 2 - y) / size;
      const c = scene.trace(scene.camera.rayFor(vx, vy), 0).clamp();
      const r = Math.floor(c.r * 255);
      const g = Math.floor(c.g * 255);
      const b = Math.floor(c.b * 255);
      sum += r + g + b;
      hash = (hash * 31 + r * 65536 + g * 256 + b) | 0;
    }
    if (y % 50 === 0) rowStr.push(String(sum));
  }
  return [sum, hash, rowStr.join(",")];
}

const result = render(buildScene(), SIZE);
console.log("raytrace", SIZE, result[0], result[1]);
console.log("rows", result[2]);
