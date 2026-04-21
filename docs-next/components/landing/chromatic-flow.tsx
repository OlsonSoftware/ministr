'use client';

import { Canvas, useFrame, useThree } from '@react-three/fiber';
import { useEffect, useMemo, useRef, useState } from 'react';
import { motion, useReducedMotion } from 'motion/react';
import * as THREE from 'three';

/**
 * ChromaticFlow — section-aligned ambient shader backdrop.
 *
 * A full-screen fragment pass driven by the *current section* rather
 * than raw scroll. Every section of the landing (Hero, Stats, Thesis,
 * …) has an explicit mood preset — warp direction, noise scale,
 * intensity, palette offset — so passing from one section to the next
 * feels like stepping between differently-lit rooms. A damped lerp
 * handles the in-between so transitions are smooth.
 *
 * Dark mode: screen blend over an ink-950 page, iris/violet/fuchsia
 * spectrum, brightening contribution.
 * Light mode: multiply blend over a near-white page, desaturated
 * iris ink, darkening contribution — so the backdrop reads as faint
 * coloured paper grain, never washed out.
 */
export function ChromaticFlow({ className = '' }: { className?: string }) {
  const reduced = useReducedMotion();
  return (
    <motion.div
      aria-hidden
      className={'chromatic-flow-mount pointer-events-none fixed inset-0 -z-10 ' + className}
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      transition={{ duration: 1.2, ease: [0.2, 0.8, 0.2, 1] }}
    >
      <Canvas
        dpr={[0.75, 1]}
        frameloop="always"
        camera={{ position: [0, 0, 1], fov: 60, near: 0.1, far: 10 }}
        gl={{
          alpha: true,
          antialias: false,
          premultipliedAlpha: false,
          powerPreference: 'low-power',
        }}
        style={{ background: 'transparent' }}
      >
        <FlowQuad reduced={!!reduced} />
      </Canvas>
    </motion.div>
  );
}

/* ------------------------------------------------------------------ */

function FlowQuad({ reduced }: { reduced: boolean }) {
  const mat = useRef<THREE.ShaderMaterial>(null);
  const { size, viewport } = useThree();

  const [sections, setSections] = useState<Element[]>([]);
  const phaseSmooth   = useRef(0);
  const phaseVel      = useRef(0);
  const phasePrev     = useRef(0);
  const themeSmooth   = useRef(0);
  const mouseTarget   = useRef<[number, number]>([0.5, 0.5]);
  const mouseSmooth   = useRef<[number, number]>([0.5, 0.5]);

  // Mouse tracking for reactive light source.
  useEffect(() => {
    const onMove = (e: PointerEvent) => {
      mouseTarget.current = [
        e.clientX / window.innerWidth,
        1 - e.clientY / window.innerHeight,
      ];
    };
    window.addEventListener('pointermove', onMove, { passive: true });
    return () => window.removeEventListener('pointermove', onMove);
  }, []);

  // Discover sections once on mount, re-discover on resize so layout
  // shifts (fonts loaded, media loaded) are picked up.
  useEffect(() => {
    const discover = () => {
      const list = Array.from(
        document.querySelectorAll<HTMLElement>('.iris-landing > section'),
      );
      setSections(list);
    };
    discover();
    const raf = requestAnimationFrame(discover);
    window.addEventListener('resize', discover);
    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener('resize', discover);
    };
  }, []);

  const uniforms = useMemo(
    () => ({
      uTime:         { value: 0 },
      uSection:      { value: 0 },
      uSectionCount: { value: 1 },
      uTheme:        { value: 0 },
      uVelocity:     { value: 0 },
      uMouse:        { value: new THREE.Vector2(0.5, 0.5) },
      uRes:          { value: new THREE.Vector2(size.width, size.height) },
    }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [],
  );

  useEffect(() => {
    uniforms.uRes.value.set(size.width, size.height);
  }, [size, uniforms.uRes]);

  useEffect(() => {
    uniforms.uSectionCount.value = Math.max(1, sections.length);
  }, [sections.length, uniforms.uSectionCount]);

  useFrame((_, delta) => {
    if (!mat.current) return;
    // Write uniforms directly on the material. The external `uniforms`
    // object passed to <shaderMaterial> seeds the initial values but
    // isn't the live reference THREE uses for uploads.
    const u = mat.current.uniforms as Record<string, { value: unknown }>;

    if (!reduced) (u.uTime.value as number) += delta;

    // Read theme from the DOM every frame — cheap boolean class
    // check, immune to MutationObserver timing races with whatever
    // theme provider is in use. Smoothed so switching cross-fades.
    const themeTarget = document.documentElement.classList.contains('dark')
      ? 1
      : 0;
    const kTheme = 1 - Math.pow(0.001, delta * 2.0);
    themeSmooth.current += (themeTarget - themeSmooth.current) * kTheme;
    u.uTheme.value = themeSmooth.current;

    const nSections = sections.length;
    if (nSections > 0) {
      const vpCenter = window.innerHeight * 0.5;

      // Find the fractional section index at the viewport center by
      // interpolating between consecutive section midpoints.
      let target = 0;
      const centers: number[] = [];
      for (let i = 0; i < nSections; i++) {
        const r = sections[i].getBoundingClientRect();
        centers.push(r.top + r.height * 0.5);
      }

      if (vpCenter <= centers[0]) {
        target = 0;
      } else if (vpCenter >= centers[nSections - 1]) {
        target = nSections - 1;
      } else {
        for (let i = 0; i < nSections - 1; i++) {
          if (vpCenter >= centers[i] && vpCenter < centers[i + 1]) {
            const span = Math.max(1, centers[i + 1] - centers[i]);
            // Linear, not S-curve: the S-curve made the phase dwell
            // at section centres and rush across boundaries, which
            // read as jarring "clicks". Linear + soft damping gives
            // a continuous drift that feels like wind through smoke.
            target = i + (vpCenter - centers[i]) / span;
            break;
          }
        }
      }

      // Slow damped lerp toward the section-aligned target.
      const k = 1 - Math.pow(0.001, delta * 1.1);
      phaseSmooth.current += (target - phaseSmooth.current) * k;
      u.uSection.value = phaseSmooth.current;

      // Scroll velocity — derivative of the smoothed phase. Drives
      // distortion + chromatic intensity in the shader, so fast
      // scrolls briefly blur into motion and settle back to calm.
      const rawVel = (phaseSmooth.current - phasePrev.current) / Math.max(delta, 1e-3);
      phasePrev.current = phaseSmooth.current;
      const kv = 1 - Math.pow(0.001, delta * 2.5);
      phaseVel.current += (rawVel - phaseVel.current) * kv;
      u.uVelocity.value = Math.abs(phaseVel.current);
    }

    // Smoothed mouse — lags the cursor so the reactive light feels
    // like an inertial glow rather than a hard tracker.
    const [mx, my] = mouseTarget.current;
    const [sx, sy] = mouseSmooth.current;
    const km = 1 - Math.pow(0.001, delta * 1.5);
    mouseSmooth.current = [sx + (mx - sx) * km, sy + (my - sy) * km];
    (u.uMouse.value as THREE.Vector2).set(
      mouseSmooth.current[0],
      mouseSmooth.current[1],
    );
  });

  const scale: [number, number, number] = [viewport.width, viewport.height, 1];

  return (
    <mesh scale={scale}>
      <planeGeometry args={[1, 1]} />
      <shaderMaterial
        ref={mat}
        uniforms={uniforms}
        vertexShader={VERT}
        fragmentShader={FRAG}
        transparent
        depthTest={false}
        depthWrite={false}
      />
    </mesh>
  );
}

/* ------------------------------------------------------------------ */

const VERT = /* glsl */ `
  varying vec2 vUv;
  void main() {
    vUv = uv;
    gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
  }
`;

const FRAG = /* glsl */ `
  precision highp float;

  varying vec2 vUv;
  uniform float uTime;
  uniform float uSection;
  uniform float uSectionCount;
  uniform float uTheme;
  uniform float uVelocity;      // smoothed |dphase/dt|, drives motion fx
  uniform vec2  uMouse;         // 0..1 normalized
  uniform vec2  uRes;

  /* ------------ noise primitives ------------ */

  float hash(vec2 p) {
    p = fract(p * vec2(123.34, 456.21));
    p += dot(p, p + 45.32);
    return fract(p.x * p.y);
  }

  float vnoise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = f * f * (3.0 - 2.0 * f);
    float a = hash(i);
    float b = hash(i + vec2(1.0, 0.0));
    float c = hash(i + vec2(0.0, 1.0));
    float d = hash(i + vec2(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
  }

  /* 3-octave fBm — enough detail when combined with parallax layers. */
  float fbm(vec2 p) {
    float s = 0.0;
    float a = 0.55;
    mat2 R = mat2(0.8, -0.6, 0.6, 0.8);
    for (int i = 0; i < 3; i++) {
      s += a * vnoise(p);
      p = R * p * 2.03 + 17.0;
      a *= 0.52;
    }
    return s;
  }

  /* Recursive domain warp (Iñigo Quílez) — three levels of warping
     produces the characteristic slow-rolling smoke shape. Returns
     (density, mid-warp, wisp-detail) so the caller can drive
     multiple visual layers off the same field. */
  vec3 smokeField(vec2 p, float t) {
    vec2 q = vec2(
      fbm(p + vec2(0.0, t * 0.35)),
      fbm(p + vec2(5.2, 1.3) - vec2(t * 0.3, 0.0))
    );
    vec2 r = vec2(
      fbm(p + 3.1 * q + vec2(1.7, 9.2) + t * 0.18),
      fbm(p + 3.1 * q + vec2(8.3, 2.8) - t * 0.14)
    );
    float v = fbm(p + 3.3 * r);
    return vec3(v, q.x, r.x);
  }

  /* ------------ iris spectrum ------------

     Two palettes — the light one is a deliberate inversion of the
     dark palette: each colour pushed toward a desaturated ink that
     multiplies onto paper without going muddy. */
  vec3 darkHue(float phase) {
    phase = fract(phase);
    vec3 iris    = vec3(0.36, 0.27, 0.85);
    vec3 violet  = vec3(0.58, 0.28, 0.95);
    vec3 fuchsia = vec3(0.92, 0.42, 0.92);
    vec3 magenta = vec3(0.72, 0.30, 0.78);
    vec3 col = mix(iris,    violet,  smoothstep(0.00, 0.33, phase));
    col      = mix(col,     fuchsia, smoothstep(0.33, 0.66, phase));
    col      = mix(col,     magenta, smoothstep(0.66, 0.95, phase));
    col      = mix(col,     iris,    smoothstep(0.92, 1.00, phase));
    return col;
  }

  vec3 lightHue(float phase) {
    phase = fract(phase);
    // Deeper, paper-safe variants: saturated but darker so they
    // read as coloured ink rather than pastel wash when used with
    // multiply blend over white. Hand-picked for OKLCH legibility.
    vec3 iris    = vec3(0.24, 0.20, 0.70);
    vec3 violet  = vec3(0.36, 0.18, 0.75);
    vec3 fuchsia = vec3(0.68, 0.25, 0.74);
    vec3 magenta = vec3(0.50, 0.18, 0.52);
    vec3 col = mix(iris,    violet,  smoothstep(0.00, 0.33, phase));
    col      = mix(col,     fuchsia, smoothstep(0.33, 0.66, phase));
    col      = mix(col,     magenta, smoothstep(0.66, 0.95, phase));
    col      = mix(col,     iris,    smoothstep(0.92, 1.00, phase));
    return col;
  }

  vec3 hueAt(float phase) {
    return mix(lightHue(phase), darkHue(phase), uTheme);
  }

  /* ------------ Henyey-Greenstein phase function ------------
     Models how light scatters through participating media. g=0 is
     isotropic; g→1 is forward-scattering (halo around the sun);
     g→-1 is back-scattering. */
  float hg(float cosTheta, float g) {
    float g2 = g * g;
    float denom = 1.0 + g2 - 2.0 * g * cosTheta;
    return (1.0 - g2) / (4.0 * 3.14159 * pow(max(denom, 1e-4), 1.5));
  }

  /* Draine phase function (2003) — bridges HG and Rayleigh.
     P_D(θ, g, α) = P_HG(θ, g) · (1 + α·cos²θ) / (1 + α·(1+2·g²)/3).
     With α=0 it reduces to HG; with g=0, α=1 it reduces to Rayleigh.
     Produces sharper forward peak and softer backscatter than plain HG. */
  float draine(float cosTheta, float g, float alpha) {
    float g2 = g * g;
    float denom = 1.0 + g2 - 2.0 * g * cosTheta;
    float hgv = (1.0 - g2) / (4.0 * 3.14159 * pow(max(denom, 1e-4), 1.5));
    float num = 1.0 + alpha * cosTheta * cosTheta;
    float norm = 1.0 + alpha * (1.0 + 2.0 * g2) / 3.0;
    return hgv * num / max(norm, 1e-4);
  }

  /* Jendersie-d'Eon 2023 HG-Draine blend — matches 95% of the real
     Mie scattering function for cloud droplet distributions, at a
     fraction of the cost of a Mie lookup. w blends HG (forward) with
     a Draine term (bridge term). */
  float mieHGDraine(float cosTheta, float g) {
    float w = 0.5;
    float a = 0.5;
    return mix(hg(cosTheta, g), draine(cosTheta, g, a), w);
  }

  /* Two-term Henyey-Greenstein — combines a forward-biased and a
     back-scattering HG, commonly used for smoke/dust where small
     particles scatter both forward and back more than plain HG. */
  float tthg(float cosTheta, float gF, float gB, float wF) {
    return wF * hg(cosTheta, gF) + (1.0 - wF) * hg(cosTheta, gB);
  }

  /* Interleaved Gradient Noise (Jimenez, Activision 2014).
     IGN(x, y) = fract(52.9829189 * fract(0.06711056*x + 0.00583715*y))
     Produces a low-discrepancy noise pattern optimised for temporal
     stability - used in Call of Duty, God of War etc. Much better
     than hash noise for dithering bands without flickering. */
  float ign(vec2 p) {
    return fract(52.9829189 * fract(dot(p, vec2(0.06711056, 0.00583715))));
  }

  /* ACES filmic tonemapping (Narkowicz 2015 fit). Maps HDR colour
     to the 0..1 display range with a gentle shoulder and toe so
     highlights keep their chroma instead of blowing out to white. */
  vec3 aces(vec3 x) {
    const float a = 2.51;
    const float b = 0.03;
    const float c = 2.43;
    const float d = 0.59;
    const float e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), 0.0, 1.0);
  }

  /* Anamorphic-lens horizontal streak — a CinemaScope-style
     elongated flare that emerges around bright light sources. Two
     sharpness scales so it reads as a streak, not a line. */
  float anamorphicStreak(vec2 d, float mul) {
    float y = exp(-abs(d.y) * 24.0 * mul);
    float x = exp(-abs(d.x) * 1.6);
    return y * x;
  }

  /* Kaliset-style iterative folding — repeatedly folds the
     coordinate back into a small domain while accumulating hits.
     The result is the filament/streak/star-forming-region texture
     that real nebulae exhibit. Used to *shape* the smoke mass into
     something that reads as celestial, not just cloud. */
  float kaliset(vec2 p, vec2 seed, int iters) {
    float acc = 0.0;
    vec2 q = p;
    for (int i = 0; i < 8; i++) {
      if (i >= iters) break;
      q = abs(q) / dot(q, q) - seed;
      acc += exp(-dot(q, q) * 1.4);
    }
    return acc;
  }

  /* Cardelli-Clayton-Mathis 1989 interstellar reddening.
     The Milky Way diffuse extinction curve A(λ)/A(V) for R_V=3.1
     evaluated at R=700nm / G=550nm / B=450nm (V-band reference):
       R: 0.751   (long wavelength, passes through dust)
       G: 1.000   (V-band reference)
       B: 1.324   (short wavelength, most absorbed)
     Blue is attenuated ~75% more than red — the canonical
     "reddening" effect of interstellar dust. A star shining through
     a dense smoke column therefore shifts toward red. */
  const vec3 CCM_EXTINCTION = vec3(0.751, 1.000, 1.324);

  /* Apply CCM reddening to an unreddened star colour given the
     local column density of intervening dust (A_V = visual-band
     extinction magnitude). exp(-A_channel * ln10 / 2.5) converts
     magnitude to flux fraction — A_V = 1 mag ≈ 40% flux loss in V. */
  vec3 reddenStar(vec3 col, float A_V) {
    vec3 A_channel = CCM_EXTINCTION * A_V;
    // magnitude to flux: f/f0 = 10^(-A/2.5), i.e. exp(-A * 0.921)
    return col * exp(-A_channel * 0.921);
  }

  /* JWST-style diffraction spikes. Stars so bright that light wraps
     around the primary mirror's sharp hexagonal edges produce an
     8-arm cross: 6 major spikes from the hexagonal segment edges
     (60° apart) plus 2 smaller spikes from the secondary mirror
     struts. We approximate with pow-of-cosine angular modulations. */
  float diffractionSpikes(vec2 offset) {
    float r = length(offset);
    if (r < 1e-5) return 0.0;
    float angle = atan(offset.y, offset.x);
    // 6 primary spikes from hexagonal primary mirror (60° = pi/3)
    float c6 = cos(angle * 3.0);
    float spike6 = pow(max(abs(c6), 0.0), 90.0);
    // 2 secondary spikes from secondary mirror struts (horizontal)
    float c2 = cos(angle);
    float spike2 = pow(max(abs(c2), 0.0), 140.0) * 0.4;
    float spikes = spike6 + spike2;
    // Radial falloff — long thin rays (1/r decay is classic
    // diffraction scaling).
    return spikes / (1.0 + r * 60.0);
  }

  /* Airy disk — the true diffraction pattern of a point source
     through a circular aperture. Intensity I(theta) proportional to
     (2*J1(x)/x)^2 where x = pi*D*sin(theta)/lambda and J1 is the
     Bessel function of the first kind. The first minimum at
     x=3.8317, first secondary maximum at x=5.1356 (1.75% of core
     intensity), second minimum at x=7.0156. We approximate J1 with
     a smooth-cosine ring envelope calibrated to hit those zeros at
     the correct radii. This is what you actually see in diffraction
     limited telescope images of bright stars. */
  float airyRings(float r) {
    // Scale so r=1 lands roughly at the first minimum (x=3.83).
    float x = r * 3.8317;
    if (x < 0.001) return 1.0;
    // Smooth approximation to (2*J1(x)/x)^2: a fast-decaying core
    // plus two ring peaks at the correct Bessel-function positions.
    float core = exp(-x * x * 0.8);
    float ring1 = exp(-pow(x - 5.14, 2.0) * 2.2) * 0.0175;
    float ring2 = exp(-pow(x - 8.42, 2.0) * 3.0) * 0.0042;
    return core + ring1 + ring2;
  }

  /* Planck black-body colour - given a temperature in Kelvin,
     returns the approximate RGB colour of that black-body emission.
     Polynomial fit to integrated CIE matching functions over the
     Planck spectrum (Chiu 2008 approximation). Defined here so
     starfield can use it. */
  vec3 planckColor(float kelvin) {
    float t = clamp(kelvin, 1000.0, 40000.0) / 100.0;
    float r, g, b;
    if (t <= 66.0) {
      r = 1.0;
      g = clamp(0.390 * log(t) - 0.631, 0.0, 1.0);
      b = (t <= 19.0) ? 0.0 : clamp(0.543 * log(t - 10.0) - 1.196, 0.0, 1.0);
    } else {
      r = clamp(1.292 * pow(t - 60.0, -0.1332), 0.0, 1.0);
      g = clamp(1.129 * pow(t - 60.0, -0.0755), 0.0, 1.0);
      b = 1.0;
    }
    return vec3(r, g, b);
  }

  /* Physically-coloured twinkling starfield with JWST-style
     diffraction spikes and magnitude-weighted rarity. */
  vec4 stars(vec2 p, float t, float density, float seedMul) {
    vec2 gp = floor(p * 30.0);
    vec2 gf = fract(p * 30.0) - 0.5;
    float h  = fract(sin(dot(gp, vec2(12.9898, 78.233) * seedMul)) * 43758.5453);
    float h2 = fract(sin(dot(gp, vec2(21.7174, 56.129) * seedMul)) * 24836.793);
    float h3 = fract(sin(dot(gp, vec2(33.4421, 92.845) * seedMul)) * 19813.221);
    if (h > 1.0 - density) {
      float starSize = mix(0.05, 0.12, fract(h * 3.7));
      // H-R main-sequence distribution: cool K/M stars common,
      // hot O/B stars rare. Temperature drawn 2500-14000 K.
      float tempK = mix(2500.0, 14000.0, pow(h2, 2.0));
      vec3  col   = planckColor(tempK);
      float twinkle = 0.55 + 0.45 * sin(t * 3.0 + h * 12.0);
      float d = length(gf);
      // True Airy-disk core + first two diffraction rings (scaled
      // to the star's apparent size). Physically correct point-
      // source response of a circular aperture — what every real
      // telescope/eye/camera actually captures.
      float airyR = d / max(starSize * 1.2, 1e-4);
      float airy = airyRings(airyR);
      // Apparent magnitude: roughly logarithmic distribution —
      // only ~5% of stars are "bright" enough to show spikes.
      float brightness = pow(h3, 6.0);  // heavy tail toward zero
      // Diffraction spikes only on the brightest stars, and even
      // then as a subtle 8-arm halo that doesn't blow up.
      float spikes = 0.0;
      if (brightness > 0.55) {
        float spikeScale = (brightness - 0.55) / 0.45;
        spikes = diffractionSpikes(gf) * spikeScale * 0.28;
      }
      float strength = (airy + spikes) * twinkle;
      return vec4(col, strength);
    }
    return vec4(0.0);
  }

  /* Hexagonal bokeh — a floating out-of-focus particle with the
     characteristic 6-sided aperture shape real lenses produce when
     stopped down. Formed by taking the intersection of 6 linear
     half-spaces rotated at 60° intervals around the particle centre.
     The soft inner fill + brighter rim approximates the typical
     bokeh look (catadioptric + spherical aberration). */
  float hexBokeh(vec2 d, float size) {
    // Rotate to canonical hexagon orientation
    float r = length(d);
    if (r > size * 1.2) return 0.0;
    float ang = atan(d.y, d.x);
    // Distance to nearest hex edge: hex is intersection of 3 stripes
    // at 60° rotations in a hex-symmetric pattern.
    float hexDist = 0.0;
    for (int i = 0; i < 3; i++) {
      float theta = float(i) * 1.04719755; // pi/3
      vec2 n = vec2(cos(theta), sin(theta));
      hexDist = max(hexDist, abs(dot(d, n)));
    }
    float inside = smoothstep(size * 1.02, size * 0.85, hexDist);
    // Brighter rim - real bokeh has a bright edge highlight
    float rim = smoothstep(size * 1.02, size * 0.95, hexDist)
              - smoothstep(size * 0.92, size * 0.85, hexDist);
    return inside * 0.55 + rim * 0.9;
  }

  /* Lens ghost bubble — soft falloff with gentle noise-perturbed
     edge so it reads as real optical glass rather than a stamped
     circle. Shape variation via a simple ellipse scale. */
  float lensGhost(vec2 p, vec2 center, float radius, float softness, vec2 ellipse) {
    vec2 d2 = (p - center) * ellipse;
    float d = length(d2);
    // gentle noise on the radius to break the geometric circle
    float nz = vnoise(p * 6.0 + center * 8.0) * 0.08;
    return smoothstep(radius + nz, radius - softness, d)
         * smoothstep(radius * 0.25, radius * 0.55, d);
  }

  float lensGhostSolid(vec2 p, vec2 center, float radius, vec2 ellipse) {
    vec2 d2 = (p - center) * ellipse;
    float nz = vnoise(p * 4.0 + center * 5.0) * 0.06;
    return smoothstep(radius + nz, 0.0, length(d2));
  }

  /* ============================================================
     Physically-based atmospheric scattering & emission primitives
     ============================================================ */

  /* Rayleigh scattering - the lambda^-4 wavelength dependency that
     gives real skies their blue cast. At R=700nm, G=550nm, B=450nm
     our per-channel coefficients (normalised to B=1) are:
       Rr = (450/700)^4 ~= 0.171
       Rg = (450/550)^4 ~= 0.449
       Rb = 1.000
     Dense regions thus turn bluer, matching real aerial perspective. */
  const vec3 RAYLEIGH_SCATTER = vec3(0.171, 0.449, 1.000);

  /* Mie scattering — forward-biased, near-achromatic. For aerosols
     (smoke particles) the Mie coefficient is roughly wavelength-
     independent but very forward-scattering (high g). */
  const vec3 MIE_SCATTER = vec3(0.95, 0.98, 1.00);

  /* Baranoski aurora emission model — three atomic emission lines:
       557.7 nm  atomic O at lower altitude   (green)
       630.0 nm  atomic O at higher altitude  (red)
       427.8 nm  N2+ ion at middle altitude   (blue)
     Normalised to peak-1 RGB values rather than spectral radiance. */
  const vec3 AURORA_GREEN = vec3(0.20, 1.00, 0.55); // 557.7 nm
  const vec3 AURORA_RED   = vec3(1.00, 0.18, 0.38); // 630.0 nm
  const vec3 AURORA_BLUE  = vec3(0.22, 0.30, 1.00); // 427.8 nm

  /* Curl-noise approximation for a divergence-free 2D flow field.
     Curl(F) = (dF/dy, -dF/dx). This is the same flow topology
     real atmospheric scientists use to approximate aurora motion. */
  vec2 curl(vec2 p, float t) {
    float e = 0.02;
    float fx = fbm(p + vec2(e, 0.0) + vec2(t, 0.0))
             - fbm(p - vec2(e, 0.0) + vec2(t, 0.0));
    float fy = fbm(p + vec2(0.0, e) + vec2(t, 0.0))
             - fbm(p - vec2(0.0, e) + vec2(t, 0.0));
    return vec2(fy, -fx) / (2.0 * e);
  }

  /* Aurora ribbon density — a vertical curtain whose lateral
     position is displaced by a curl-noise flow field over time.
     Height above the curtain base controls which emission line
     dominates (low=green, mid=blue, high=red) following the real
     altitude-emission relationship. */
  vec4 aurora(vec2 p, float t) {
    // Curl-deformed lateral displacement — the characteristic
    // wavy northern-lights motion.
    vec2 c = curl(p * 0.8 + vec2(0.0, t * 0.1), t * 0.12);
    float xShift = c.x * 0.6;
    // A second slow curtain phase shifts the curtain bodily.
    xShift += sin(p.y * 0.8 + t * 0.15) * 0.3;

    float x = p.x - xShift;

    // Curtain density with vertical falloff to nothing, and
    // horizontal thinness — the characteristic sheet look.
    float sheet = exp(-x * x * 3.5);

    // Band modulation — several parallel curtains.
    sheet *= 0.4 + 0.6 * smoothstep(-0.3, 0.0, sin(x * 6.0 + t * 0.5));

    // Vertical shape — soft bottom, harder top falloff like a
    // real curtain hanging from the ionosphere.
    float yTop = 0.6;
    float yBot = -0.3;
    float height = smoothstep(yBot - 0.3, yBot + 0.2, p.y)
                 * smoothstep(yTop + 0.4, yTop - 0.1, p.y);

    float density = sheet * height;

    // Altitude-dependent emission: green at low altitude (p.y near
    // yBot), blue in the middle, red at the top. This IS the
    // Baranoski three-line model expressed as RGB interpolation.
    float alt = clamp((p.y - yBot) / (yTop - yBot), 0.0, 1.0);
    vec3 emission = mix(AURORA_GREEN, AURORA_BLUE, smoothstep(0.15, 0.55, alt));
    emission      = mix(emission, AURORA_RED,  smoothstep(0.55, 0.95, alt));

    return vec4(emission, density);
  }

  /* Cauchy spectral index of refraction - real glass has a small
     wavelength-dependent IOR: n(lambda) = A + B/lambda^2. This is
     what makes a prism split white light into rainbow colours. We
     encode the per-channel extra displacement for our three RGB
     wavelengths. */
  const vec3 CAUCHY_DISP = vec3(-0.012, 0.000, 0.020);

  /* Chappuis ozone absorption band - real atmospheric O3 absorbs
     in a broad band centred around 600 nm, giving thick-path
     horizons their characteristic subtle red/cyan cast. Dominant
     contribution at sunset. Per-channel extinction normalised so
     green is strongest (matches real spectral fit). */
  const vec3 OZONE_CHAPPUIS = vec3(0.65, 1.88, 0.08);

  /* Hillaire 2020 multi-scattering approximation - given single-
     scattering radiance L_ss and an extinction ratio sigmaS/sigmaT,
     the analytic series expansion gives:
       L_ms = L_ss * f_ms / (1 - f_ms * albedo)
     where f_ms is a dimensionless isotropic scattering term and
     albedo is the scattering/extinction ratio. A cheap, energy-
     conserving way to add the bright haze real atmospheres have. */
  vec3 hillaireMS(vec3 lss, vec3 albedo, float fms) {
    return lss * fms / max(vec3(1.0) - fms * albedo, vec3(1e-4));
  }

  /* Wrenninge 2015 (SIGGRAPH "Oz: The Great and Powerful") anisotropic
     multi-scattering. Instead of assuming multi-scatter is isotropic
     (Hillaire), Wrenninge noticed that each successive scattering
     event preserves SOME directional bias from the previous one.
     The energy-conserving series becomes:
       L_n = albedo^n * phase(cosTheta, g*attenuation^n) * L_ss
     Summed across n=1..N with attenuation < 1 per bounce. Produces
     the characteristic bright "silver lining" on cloud edges facing
     the sun - an effect isotropic MS cannot reproduce. */
  vec3 wrenningeMS(vec3 lss, vec3 albedo, float gBase, float cosTheta) {
    vec3 acc = vec3(0.0);
    vec3 alb = vec3(1.0);
    float g = gBase;
    float atten = 0.5;
    for (int i = 0; i < 4; i++) {
      alb *= albedo;
      g *= atten;
      acc += alb * hg(cosTheta, g) * lss;
    }
    return acc;
  }

  /* Chapman atmospheric function (Smith 1972) - the correct path-
     length integral through an exponential atmosphere as a function
     of zenith angle. The Chapman function Ch(chi, X) approximates:
       path_length / vertical_scale = sec(chi) near zenith
       but grows without bound only at true grazing angles (chi=pi/2).
     We use a Rezzolla 2002 fast polynomial approximation. X is the
     normalised optical depth from observer to space through zenith;
     for the visual atmosphere X ~ 8 (scale height). */
  float chapman(float zenithCos, float X) {
    // Rezzolla fit: accurate to <1% for chi in [0, pi/2]
    float chi = acos(clamp(zenithCos, -1.0, 1.0));
    float sinChi = sin(chi);
    // Standard Chapman approximation: sqrt(pi*X/2) * erfc-like term
    // We linearise to sec(chi) capped for near-grazing rays.
    float sec = 1.0 / max(abs(zenithCos), 0.03);
    // Limb effect: grazing rays through spherical shell have finite
    // path length ~ sqrt(2*pi*R*h) not infinite sec.
    float limb = sqrt(X * 3.14159 * 0.5);
    return mix(sec, limb, smoothstep(0.3, 0.0, abs(zenithCos)));
  }

  /* Atmospheric transmittance along view direction through an
     exponential atmosphere with per-channel Rayleigh + Mie + Ozone
     extinction. Approximates the Bruneton 2008 T(mu) LUT as a
     closed-form expression suitable for a fragment shader. */
  vec3 atmoTransmittance(float viewCosZenith, float density) {
    float pathLen = chapman(viewCosZenith, 8.0) * density;
    vec3 tauR = RAYLEIGH_SCATTER * 0.35;
    vec3 tauM = MIE_SCATTER      * 0.14;
    vec3 tauO = OZONE_CHAPPUIS   * 0.03;
    vec3 tau  = (tauR + tauM + tauO) * pathLen;
    return exp(-tau);
  }

  /* Sun-dog / 22° parhelion — iridescent bright spot at exactly
     21.83° from the sun caused by light refracting through the 60°
     prism faces of hexagonal ice crystals in high cirrus. The angle
     is fixed by Snell's law through ice (n=1.31); minimum deflection
     is 22°. Real sundogs show red edge toward the sun (lower-n ->
     less deflection) and blue edge outward. */
  vec3 sundog(vec2 offset, float radius) {
    float r = length(offset);
    // Thin ring at the 22° radius with 60° angular width (cirrus
    // distribution isn't uniform).
    float ringDist = abs(r - radius);
    float ring = exp(-ringDist * ringDist * 140.0);
    // Angular envelope — sundogs are strongest in the horizontal
    // plane through the sun, so prefer small |y|.
    float angular = exp(-offset.y * offset.y * 18.0);
    // Spectral split: inner edge red, outer edge blue (real prism
    // dispersion through hexagonal ice).
    float edgeT = smoothstep(radius * 0.99, radius * 1.02, r);
    vec3 spectrum = mix(
      vec3(1.00, 0.72, 0.52),   // warm red (inner edge)
      vec3(0.62, 0.82, 1.00),   // cool blue (outer edge)
      edgeT
    );
    return spectrum * ring * angular;
  }

  /* ------------ per-section mood ------------

     Everything varies *monotonically* with the section index so
     scrolling feels like drifting through a single continuous
     volume — no oscillations that would cause warp samples to
     swing back and forth at adjacent sections. */
  struct Mood {
    vec2  off;
    float scale;
    float intensity;
    float hue;
  };

  Mood moodAt(float idx) {
    Mood m;
    // Slow monotonic drift of the sampling window — the smoke
    // volume pans diagonally as you scroll through sections.
    m.off       = vec2(idx * 0.55, -idx * 1.25);
    m.scale     = 1.0 + idx * 0.04;          // very gentle zoom in
    m.intensity = 0.95 - idx * 0.02;         // tiny ease-out
    m.hue       = idx * 0.18;                 // slow walk through spectrum
    return m;
  }

  /* ------------ main ------------ */

  void main() {
    vec2 uv = vUv;
    vec2 p  = (uv - 0.5) * vec2(uRes.x / max(uRes.y, 1.0), 1.0);

    float t   = uTime * 0.05;
    float idx = uSection;

    Mood mood = moodAt(idx);

    /* Aspect-corrected mouse position in the same coordinate space
       as p — acts as the world-space position of a soft inertial
       light source. */
    vec2 mouseP = (uMouse - 0.5) * vec2(uRes.x / max(uRes.y, 1.0), 1.0);

    /* Schlieren heat-shimmer — real optics: light rays bend when
       travelling through media of varying refractive index, and air
       density (hence IOR) varies with temperature. High-speed
       shadowgraph photography visualises this as a shimmer. We
       approximate by sampling the density gradient at p and
       displacing the *scene-space* sample position along -grad(rho).
       This is exactly the ray-bending formula dr/ds = -grad(n)/n,
       linearised for a thin layer.

       Crucially, lens/film artifacts (bokeh hexagons, lens ghosts,
       anamorphic streaks, halation, sunburst rays) are imprinted by
       the camera aperture on its own sensor — they live in SCREEN
       space and don't bend with the scene. So we preserve pLens for
       those, and only displace p (which feeds scene content: smoke,
       stars, aurora, airglow, nebula). */
    vec2 pLens = p;
    float eps = 0.015;
    float gX = smokeField(p * 1.1 + vec2(eps, 0.0) + mood.off, t + idx * 0.35).x
             - smokeField(p * 1.1 - vec2(eps, 0.0) + mood.off, t + idx * 0.35).x;
    float gY = smokeField(p * 1.1 + vec2(0.0, eps) + mood.off, t + idx * 0.35).x
             - smokeField(p * 1.1 - vec2(0.0, eps) + mood.off, t + idx * 0.35).x;
    vec2 schlieren = vec2(gX, gY) / (2.0 * eps);
    // Ray bending amplitude - small so the composition stays stable
    // but visible enough to catch as "shimmer" near bright features.
    p -= schlieren * 0.008;

    /* Scroll velocity drives subtle motion-blur-style UV offset and
       extra chromatic separation. Clamped so fast scroll never
       destabilises the composition. */
    float vel = clamp(uVelocity * 0.35, 0.0, 0.8);
    vec2  velDir = normalize(vec2(0.15, -1.0));
    p += velDir * vel * 0.05;

    /* Three parallax layers = three "depths" through the volume.
       Each is the same smokeField sampled with different scale,
       offset, and time to produce independent motion. Accumulated
       back-to-front with Beer-Lambert transmittance so dense
       foreground smoke occludes the background — the heart of
       volumetric realism. */
    vec2 base = p * (0.9 * mood.scale) + mood.off;

    // Layer 0 — distant, slow, diffuse
    vec3 f0 = smokeField(base * 0.55 + vec2(-idx * 0.40, -t * 0.15), t * 0.55 + idx * 0.20);
    // Layer 1 — mid depth
    vec3 f1 = smokeField(base * 0.95 + vec2( idx * 0.30, -t * 0.32), t * 0.85 + idx * 0.35);
    // Layer 2 — near, sharp wisps, faster drift
    vec3 f2 = smokeField(base * 1.45 + vec2( idx * 0.10, -t * 0.55), t * 1.25 + idx * 0.55);

    float d0 = f0.x;
    float d1 = f1.x;
    float d2 = f2.x;

    /* Per-layer hue: background slightly cooler, foreground warmer,
       like atmospheric perspective. */
    vec3 hueB = hueAt(mood.hue - 0.05);
    vec3 hueM = hueAt(mood.hue);
    vec3 hueF = hueAt(mood.hue + 0.08 + d2 * 0.10);

    /* Beer-Lambert accumulation with SPECTRAL (per-channel)
       extinction — Rayleigh scattering blues out short wavelengths
       more than long ones, so dense regions of the volume take on
       atmospheric-blue character, while thin regions preserve
       warmer tones. This is exactly why real fog/clouds look blue
       when thick and white when thin. */
    vec3 T = vec3(1.0);  // per-channel transmittance
    vec3 lit = vec3(0.0);

    // Back layer — mostly Rayleigh at distance (aerial perspective)
    // plus a dash of Chappuis ozone for correct horizon tint.
    float densB = smoothstep(0.10, 0.95, d0) * mood.intensity;
    lit += hueB * densB * 0.30 * T;
    vec3 extB = mix(MIE_SCATTER, RAYLEIGH_SCATTER, 0.60) + OZONE_CHAPPUIS * 0.04;
    T *= exp(-densB * 1.10 * extB);

    // Mid layer — mixed Rayleigh + Mie
    float densM = smoothstep(0.08, 0.95, d1) * mood.intensity;
    lit += hueM * densM * 0.45 * T;
    vec3 extM = mix(MIE_SCATTER, RAYLEIGH_SCATTER, 0.35) + OZONE_CHAPPUIS * 0.02;
    T *= exp(-densM * 1.35 * extM);

    // Front layer — mostly Mie (thick aerosols)
    float densF = smoothstep(0.05, 0.95, d2) * mood.intensity;
    lit += hueF * densF * 0.55 * T;
    vec3 extF = mix(MIE_SCATTER, RAYLEIGH_SCATTER, 0.15);
    T *= exp(-densF * 1.55 * extF);

    /* Hillaire multi-scattering approximation term - adds the soft,
       bright haze of many-bounce light paths that real atmospheres
       exhibit. Without this, dense smoke looks artificially dark. */
    vec3 albedo = vec3(0.85, 0.88, 0.93); // slightly blue, as Mie+Rayleigh mix
    float fms = 0.18;                     // isotropic multi-scatter fraction
    vec3 ms = hillaireMS(lit, albedo, fms);
    lit += ms * 0.40;

    /* Chapman atmospheric transmittance & sky gradient - Bruneton-
       style. Look direction cosine vs zenith drives per-channel
       extinction through an exponential atmosphere. Near the zenith
       (centre of frame, looking "up") the sky is thin and blue.
       Toward the horizon (screen edges), the path is much longer
       and shorter wavelengths (blue) get absorbed/scattered out,
       leaving the characteristic warm sunset gradient. */
    float viewCosZen = clamp(0.7 + p.y * 0.4, 0.05, 1.0);
    // Luminance of the spectral transmittance so far — proxy for
    // how much of the page is currently "cloaked" by smoke.
    float Tlum0 = dot(T, vec3(0.2126, 0.7152, 0.0722));
    float atmosDensity = 0.65 + 0.35 * (1.0 - Tlum0);
    vec3 Tatmos = atmoTransmittance(viewCosZen, atmosDensity);
    // Soft blue-zenith / warm-horizon scrim added to the atmosphere.
    // The warmth is physically the residue after blue has been
    // scattered out along the long grazing path.
    vec3 skyZenith  = mix(vec3(0.18, 0.28, 0.62), vec3(0.30, 0.22, 0.48), uTheme);
    vec3 skyHorizon = mix(vec3(1.00, 0.72, 0.48), vec3(0.62, 0.35, 0.48), uTheme);
    vec3 skyGrad = mix(skyHorizon, skyZenith, viewCosZen);
    lit += skyGrad * (vec3(1.0) - Tatmos) * 0.14;

    /* Forward-scatter / edge glow — peaks where smoke is moderately
       dense, approximating light backlighting a thin cloud edge. */
    float edgeB = densB * (1.0 - densB) * 4.0;
    float edgeF = densF * (1.0 - densF) * 4.0;
    lit += hueF * edgeF * 0.35 + hueM * edgeB * 0.18;

    /* Wrenninge 2015 anisotropic multi-scattering — on top of the
       isotropic Hillaire term, add a forward-biased MS series that
       preserves directional memory across successive scatter
       events. Produces the signature "silver lining" where a cloud
       edge faces the sun: the near-forward bounces light up the
       thinnest-most-lit slivers of the volume. We feed it the
       view-sun geometry computed further below, but since we need
       it here, compute a simple forward-direction cosine now. */
    vec2 viewDirMS = -normalize(p + vec2(0.0, 0.0001));
    // Sun direction target; we don't have rayDir in this block yet
    // so approximate using the page-level sun location.
    vec2 sunLocalEarly = vec2(0.2, 1.0);  // sun roughly overhead
    float cosThetaMS = dot(viewDirMS, normalize(sunLocalEarly));
    vec3 msAniso = wrenningeMS(vec3(densF * 0.6 + densM * 0.4), albedo, 0.72, cosThetaMS);
    lit += hueF * msAniso * 0.18;

    /* ---- Godrays / volumetric light shafts ----
       The sun is a CELESTIAL body drifting along its own
       deterministic path, pinned ABOVE the viewport so a 2D phase
       function never produces a sun-to-viewer alignment beam. */
    float sunDrift = uTime * 0.03;
    vec2 sunPos = vec2(
      sin(sunDrift) * 0.55 + cos(sunDrift * 0.63) * 0.25,
      // Stay well above the visible area so view-direction alignment
      // never converges inside the frame (which would produce a
      // visible light beam sweeping as the sun "moves").
      1.15 + sin(sunDrift * 0.4) * 0.05
    );

    vec2 toSun = sunPos - p;
    float sunDist = length(toSun);
    vec2 rayDir = toSun / max(sunDist, 1e-4);

    /* IGN-jittered shadow raymarching (Muth 2026, inherits from
       Jimenez 2014). Starting each ray at a blue-noise-distributed
       sub-pixel offset t0 in [0, stepLen) kills the visible
       ringing/banding that fixed-offset raymarching produces over
       motion. Temporally stable: IGN sequence stays low-discrepancy
       even with time jitter, so no "crawling ants" flicker. */
    float shaft = 0.0;
    const int SHAFT_STEPS = 8;
    float stepLen = min(sunDist, 1.3) / float(SHAFT_STEPS);
    float jitter = ign(gl_FragCoord.xy + vec2(uTime * 60.0, 0.0));
    for (int i = 0; i < SHAFT_STEPS; i++) {
      float ti = (float(i) + jitter) * stepLen;
      vec2 sp = p + rayDir * ti;
      float d = smokeField(sp * 0.9 * mood.scale + mood.off, t + idx * 0.35).x;
      shaft += (1.0 - smoothstep(0.1, 0.8, d));
    }
    shaft /= float(SHAFT_STEPS);

    /* Jendersie-d'Eon 2023 HG-Draine blend — matches real Mie
       scattering for cloud-droplet-sized aerosols 95% of the way.
       Physically more accurate than plain HG, with a sharper
       forward peak and softer back scatter. Plus a two-term HG
       for the coarser smoke (dust particles scatter both ways). */
    vec2 viewDir = -normalize(p + vec2(0.0, 0.0001));
    float cosTheta = dot(viewDir, rayDir);
    float phaseMie = mieHGDraine(cosTheta, 0.72);
    float phaseSmoke = tthg(cosTheta, 0.68, -0.12, 0.85);
    float phase = mix(phaseSmoke, phaseMie, 0.55);
    float scatter = phase * shaft;

    // Shaft brightness falls off with distance from sun.
    float shaftFalloff = smoothstep(1.9, 0.0, sunDist);
    lit += hueF * scatter * 0.30 * shaftFalloff;

    /* Bloom core — soft HDR halo around the sun itself. Additive,
       saturated, kept subtle so it reads as a light source blooming
       through atmosphere rather than a floodlight. */
    float core = exp(-sunDist * 2.8);
    float bloom = exp(-sunDist * 0.95) * 0.18;
    lit += (hueF * 1.2) * (core * 0.18 + bloom * 0.10);

    /* Kodak 5219 film halation — on real cinematic stock, intense
       highlights penetrate the emulsion layers, scatter off the
       film base, and bleed back through before the anti-halation
       backing fully absorbs them. The result is a characteristic
       *wide red-biased* glow around bright features that's
       fundamentally asymmetric: it's much wider in red than in
       green or blue because red light penetrates deeper into the
       emulsion (longer wavelength = less scattering). This single
       effect is the visual signature of film stock vs. digital.
       Measured in LENS space — emulsion halation is a sensor-side
       artifact, independent of scene-space refraction. */
    float sunDistLens = length(sunPos - pLens);
    float halationCore = exp(-sunDistLens * 1.8);
    float halationWide = exp(-sunDistLens * 0.55);
    float shaftFalloffLens = smoothstep(1.9, 0.0, sunDistLens);
    vec3 halationRGB = vec3(halationWide * 0.22, halationWide * 0.06, halationWide * 0.02)
                     + vec3(halationCore * 0.10, halationCore * 0.03, 0.0);
    lit += halationRGB * shaftFalloffLens;

    /* Anamorphic lens streaks — horizontal CinemaScope flares from
       the sun. Two widths for a bright core and soft tail. These
       are aperture/sensor artifacts, so they track the UNDISPLACED
       screen-space sun position (pLens), not the refracted one. */
    vec2 sunLocal = pLens - sunPos;
    float streakA = anamorphicStreak(sunLocal, 1.0) * 0.22;
    float streakB = anamorphicStreak(sunLocal, 0.45) * 0.08;
    vec3 streakHue = mix(hueF, vec3(0.95, 0.75, 1.0), 0.25);
    lit += streakHue * (streakA + streakB) * shaftFalloff;

    /* Sunburst rays — the rigid "perfect starburst" pattern is
       broken by modulating the angular frequency with a noise
       sample in angle-space and giving each wave its own time
       drift. Result: a handful of organic, uneven, flickering rays
       that look like atmosphere scintillation, not stamped geometry. */
    float theta = atan(sunLocal.y, sunLocal.x);
    // per-angle noise: sample fBm in 1D angle space so the ray
    // count isn't globally uniform — some directions have thick
    // shafts, others are empty
    float angleJitter = fbm(vec2(theta * 2.6, uTime * 0.08));
    float angleMod    = fbm(vec2(theta * 1.4 + 3.7, uTime * 0.05));
    float freqA = 9.0 + 4.0 * angleJitter;
    float freqB = 5.0 + 3.0 * angleMod;
    float rays = 0.0;
    rays += pow(0.5 + 0.5 * sin(theta * freqA + uTime * 0.18 + angleJitter * 6.0), 18.0)
          * (0.6 + 0.4 * angleMod);
    rays += pow(0.5 + 0.5 * sin(theta * freqB - uTime * 0.11 + angleMod * 4.0), 14.0)
          * (0.4 + 0.3 * angleJitter) * 0.55;
    // Small high-frequency flicker on top — the whole ray pattern
    // breathes on a long cycle so it never sits still.
    rays *= 0.75 + 0.25 * sin(uTime * 0.9 + angleJitter * 8.0);
    float rayFade = exp(-sunDist * 0.9) * shaftFalloff;
    lit += streakHue * rays * rayFade * 0.14;

    /* Lens ghost bubbles — a constellation of soft circular
       artifacts marching along the line from the sun through the
       screen centre. Each one is tinted a step further along the
       iris spectrum, mimicking how real multi-element lens coatings
       create a rainbow chain of ghosts. */
    vec2 ghostVec = -sunPos; // sun → centre direction (centre is 0)
    // Ghosts are lens-coating artifacts in SCREEN space — the
    // hexagon/chain of them is imprinted by the lens elements,
    // not warped by scene refraction. Use pLens.
    vec2 ghostBase = pLens;
    vec3 ghostColor = vec3(0.0);
    vec2 ell1 = vec2(1.0, 1.08);
    vec2 ell2 = vec2(1.12, 1.0);
    vec2 ell3 = vec2(0.95, 1.05);
    vec2 ell4 = vec2(1.0, 0.92);
    float ghostAnim = 0.85 + 0.15 * sin(uTime * 0.3);
    // Cauchy spectral dispersion: sample each ghost at a slightly
    // different position per RGB channel. Real glass elements split
    // wavelengths because n(lambda) varies - we're simulating that
    // physical prism behaviour here for proper rainbow fringing.
    vec2 ghostR = ghostBase + ghostVec * CAUCHY_DISP.r;
    vec2 ghostG = ghostBase + ghostVec * CAUCHY_DISP.g;
    vec2 ghostB = ghostBase + ghostVec * CAUCHY_DISP.b;

    // ghost 1 — solid disk, tint cooler
    vec2 g1 = sunPos + ghostVec * 0.45;
    vec3 h1 = vec3(
      lensGhostSolid(ghostR, g1, 0.12, ell1),
      lensGhostSolid(ghostG, g1, 0.12, ell1),
      lensGhostSolid(ghostB, g1, 0.12, ell1)
    ) * 0.28 * ghostAnim;
    ghostColor += hueAt(mood.hue + 0.05) * h1;

    // ghost 2 — ring, spectral fringing visible on the rim
    vec2 g2 = sunPos + ghostVec * 0.80;
    vec3 h2 = vec3(
      lensGhost(ghostR, g2, 0.22, 0.09, ell2),
      lensGhost(ghostG, g2, 0.22, 0.09, ell2),
      lensGhost(ghostB, g2, 0.22, 0.09, ell2)
    ) * 0.24;
    ghostColor += hueAt(mood.hue + 0.15) * h2;

    // ghost 3 — small bright solid, tinted warmer
    vec2 g3 = sunPos + ghostVec * 1.15;
    vec3 h3 = vec3(
      lensGhostSolid(ghostR, g3, 0.075, ell3),
      lensGhostSolid(ghostG, g3, 0.075, ell3),
      lensGhostSolid(ghostB, g3, 0.075, ell3)
    ) * 0.34 * ghostAnim;
    ghostColor += hueAt(mood.hue + 0.30) * h3;

    // ghost 4 — large soft ring
    vec2 g4 = sunPos + ghostVec * 1.55;
    vec3 h4 = vec3(
      lensGhost(ghostR, g4, 0.35, 0.15, ell4),
      lensGhost(ghostG, g4, 0.35, 0.15, ell4),
      lensGhost(ghostB, g4, 0.35, 0.15, ell4)
    ) * 0.14;
    ghostColor += hueAt(mood.hue + 0.45) * h4;
    // Density-occlude the ghosts a touch so they dim where smoke
    // is thick. Using the luminance of the per-channel transmittance
    // so the occlusion respects the atmosphere's overall opacity.
    float Tlum = dot(T, vec3(0.2126, 0.7152, 0.0722)); // Rec-709 luma
    ghostColor *= 0.6 + 0.4 * Tlum;
    lit += ghostColor * shaftFalloff;

    /* 22° sun-dog parhelia — lens-space (they sit on the screen at
       the angular offset from the sun set by hexagonal ice-crystal
       refraction). The 22° halo radius on our normalised coord
       system is ~0.38 in p-space (calibrated empirically so the
       ring just kisses the top edge when sun is slightly above
       frame). Shown only when sun and halo both lie within the
       field of view of the pixel we're shading — in practice the
       LEFT and RIGHT intersections appear in the upper corners.
       Faint by design: real sundogs are a ~10% brightness effect
       over clear sky. */
    vec2 sundogOff = pLens - sunPos;
    vec3 sundogCol = sundog(sundogOff, 0.36) * 0.40;
    // Cirrus is thin, so sundogs dim where smoke is thick.
    sundogCol *= Tlum;
    lit += sundogCol * shaftFalloffLens;

    /* Noctilucent clouds — silver-blue polar mesospheric clouds
       at ~80 km altitude catch sunlight after local sunset. They
       appear as thin, wavy, luminous ribbons near the upper limb
       of the sky. We render them as a narrow fBm-modulated band
       high in the frame, lit primarily by a blue-biased spectrum
       (typical of high-altitude ice crystal scatter). */
    float nlcBand = exp(-pow((p.y - 0.40) * 4.2, 2.0));
    float nlcTex  = fbm(p * vec2(2.2, 8.0) + vec2(uTime * 0.04, 0.0));
    nlcTex = smoothstep(0.45, 0.85, nlcTex);
    vec3 nlcCol = vec3(0.78, 0.88, 1.00) * nlcBand * nlcTex;
    lit += nlcCol * 0.09 * Tlum;

    /* Counter-crepuscular convergence — when light shafts travel
       across the sky from the sun, they *appear* to converge at
       the anti-solar point (opposite the sun) due to linear-
       perspective foreshortening. The real physical effect seen
       from aircraft and photographed by Minnaert: a subtle halo
       at the anti-solar point. Our sun is above-frame; anti-solar
       point is below. Adds a warm convergence at lower frame. */
    vec2 antisun = -sunPos;
    float antisunDist = length(p - antisun);
    float antisunGlow = exp(-antisunDist * 2.2) * 0.12;
    lit += hueF * antisunGlow * shaftFalloff;

    /* ---- Physically-based aurora ribbon (Baranoski emission) ----
       A vertical curtain in the upper viewport with altitude-driven
       atomic emission (557nm green / 427nm blue / 630nm red). The
       curtain is deformed by a 2D curl-noise flow field, producing
       the characteristic divergence-free swirling motion real
       auroras exhibit. */
    vec4 aur = aurora(p + vec2(idx * 0.15, 0.0), uTime);
    // Restrict to the upper viewport and soften toward screen edges
    // using a local radial metric (falloff proper is computed below).
    float aurMask = smoothstep(-0.2, 0.6, p.y);
    float aurRadial = smoothstep(1.8, 0.3, length(p));
    aurMask *= mix(0.4, 1.0, aurRadial);
    lit += aur.rgb * aur.a * aurMask * 0.55;

    /* Airglow — the faint green 557.7 nm chemiluminescence of
       recombining atomic oxygen at ~90 km altitude. Astronauts on
       the ISS photograph this as a continuous horizontal band
       hugging Earth's limb. We place a thin altitude band in the
       upper-mid viewport with a gentle fBm modulation to suggest
       the localized gravity waves that sculpt real airglow. Same
       emission colour as the low-altitude aurora line because they
       are the exact same atomic transition. */
    float airglowBand = exp(-pow((p.y - 0.25) * 3.2, 2.0));
    float airglowMod  = 0.5 + 0.5 * fbm(p * vec2(1.2, 4.0) + vec2(uTime * 0.02, 0.0));
    lit += AURORA_GREEN * airglowBand * airglowMod * 0.08 * Tlum;

    /* Kaliset nebula folding — shapes the smoke's brightest regions
       into the filamentary structure of a real nebula. We compute
       a kaliset hit count at a slow-moving coordinate and use it
       as an *additional* emission term, concentrated where the
       smoke mass already has some density. Makes deep-field areas
       of the shader look like star-forming regions. */
    vec2 kp = p * 0.6 + vec2(idx * 0.2, -t * 0.25);
    float kali = kaliset(kp, vec2(0.92 + 0.05 * sin(uTime * 0.04),
                                  0.88 + 0.05 * cos(uTime * 0.03)), 6);
    float nebula = smoothstep(0.0, 4.5, kali) * smoothstep(0.0, 1.0, densM + densF);
    lit += hueF * nebula * 0.22;

    /* Planckian starfield with Airy disks, JWST diffraction spikes,
       CCM interstellar reddening, and atmospheric scintillation.

       Stellar parallax: the near layer drifts faster than the far
       layer for the same scroll velocity, producing real 3D depth
       cues. Velocity-vector-aligned so the parallax is
       directionally meaningful, not an abstract zoom.

       Relativistic aberration: at nonzero scroll velocity, star
       positions get pulled forward along the direction of travel
       per the special-relativity formula cos(theta') = (cos(theta)
       + beta) / (1 + beta*cos(theta)). We approximate with a small
       angular bias forward. */
    float parallaxFar  = 0.05 + vel * 0.15;
    float parallaxNear = 0.10 + vel * 0.45;
    vec2 aberDir = normalize(velDir);
    float aberStrength = vel * 0.08;  // subtle - just a pull toward the motion axis
    vec2 aberOff = aberDir * aberStrength;

    vec4 starFar  = stars(p * 1.2 + vec2(idx * parallaxFar,  -t * 0.06) + aberOff * 0.4, uTime, 0.010, 1.0);
    vec4 starNear = stars(p * 0.85 - vec2(idx * parallaxNear, t * 0.12) - aberOff,       uTime, 0.006, 1.7);
    float starMask = 0.35 + 0.65 * Tlum;
    // scintillation: low-frequency turbulence * high-frequency flicker
    float scintLow  = fbm(p * 2.5 + vec2(uTime * 0.5, 0.0));
    float scintHigh = ign(gl_FragCoord.xy + vec2(uTime * 60.0, 0.0));
    float scint = 0.7 + 0.3 * scintLow + 0.1 * (scintHigh - 0.5) * 2.0;
    // Cardelli-Clayton-Mathis reddening: stars behind denser smoke
    // get their blue light absorbed more than red. The visual-band
    // extinction A_V proxy is the local column density of the mid
    // smoke layer, scaled to magnitudes.
    float A_V = densM * 1.6 + densF * 1.2;
    vec3 starFarCol  = reddenStar(starFar.rgb, A_V);
    vec3 starNearCol = reddenStar(starNear.rgb, A_V * 0.6);
    lit += starFarCol  * starFar.a  * 0.55 * starMask * scint;
    lit += starNearCol * starNear.a * 0.95 * starMask * scint;

    /* Hexagonal bokeh particles — floating out-of-focus dust motes
       that catch the sun. Each particle is a hexagonal aperture
       approximation (real stopped-down lens bokeh). Positioned on
       a slow-drifting stochastic grid so they read as dust, not
       stamped confetti.

       Bokeh shape is stamped by the aperture in SCREEN space, so
       the hexagon never deforms with scene-space schlieren.

       Scroll parallax: dust on/near the lens is the FASTEST-moving
       layer in a camera-movement parallax stack (closer to lens =
       bigger apparent translation per unit camera motion). Section
       index drives base translation; velocity pushes laterally
       along the scroll direction. Each particle has its own
       parallax depth (seed3) so the cloud of motes reads as a
       volume at different distances from the lens, not a flat
       wallpaper. */
    vec3 bokehCol = vec3(0.0);
    for (int i = 0; i < 6; i++) {
      float fi = float(i);
      float seed1 = hash(vec2(fi * 13.37, 7.0));
      float seed2 = hash(vec2(fi * 9.11, 21.0));
      float seed3 = hash(vec2(fi * 4.27, 41.0));      // per-particle depth
      float depth = 0.6 + 1.3 * seed3;                // 0.6 (far) .. 1.9 (near)
      float driftX = sin(uTime * (0.05 + seed1 * 0.03) + fi * 2.1) * 0.7;
      float driftY = cos(uTime * (0.04 + seed2 * 0.03) + fi * 1.7) * 0.4;
      // Section-index parallax — base drift inversely with depth so
      // near particles fly through faster than far ones.
      vec2 scrollShift = vec2(-idx * 0.28, idx * 0.18) * depth;
      // Velocity kick along the scroll direction — a quick scroll
      // smears the dust laterally just as a whip-pan would with a
      // real handheld camera.
      vec2 velKick = velDir * vel * 0.55 * depth;
      vec2 bpos = vec2(seed1 * 2.0 - 1.0, seed2 * 1.2 - 0.3)
                + vec2(driftX, driftY)
                + scrollShift
                + velKick;
      // Size subtly breathes with depth — nearer bokeh is bigger.
      float bsize = (0.035 + 0.025 * seed1) * mix(0.85, 1.2, seed3);
      vec2 bd = pLens - bpos;
      float bokeh = hexBokeh(bd, bsize);
      // Bokeh brightness falls off with distance from sun (out-of-
      // focus dust only shows where there's bright backlight).
      float bLight = exp(-length(bpos - sunPos) * 0.55) * shaftFalloffLens;
      // Atmospheric occlusion - thick smoke dims the mote
      float bOcc = mix(0.3, 1.0, Tlum);
      bokehCol += mix(hueF, vec3(1.0, 0.92, 0.85), 0.4) * bokeh * bLight * bOcc * 0.35;
    }
    lit += bokehCol;

    /* Cursor presence — a small, soft, independent light that
       travels with the pointer. Not the sun, not a lens flare —
       it's the feeling of a warm phosphor close to your finger,
       lifting the smoke slightly where you're pointing. Kept low
       so it never steals focus from the copy. */
    float mouseDist = length(p - mouseP);
    float mouseGlow = exp(-mouseDist * 3.0);
    lit += hueF * mouseGlow * (0.06 + 0.12 * vel);
    // A wider, dimmer halo that breathes — gives the cursor a
    // sense of parallax depth without being a hard spotlight.
    float mouseHalo = exp(-mouseDist * 1.1) * (0.5 + 0.5 * sin(uTime * 0.7));
    lit += hueF * mouseHalo * 0.020;

    /* Chromatic dispersion — scroll velocity cranks the wavelength
       split so quick scrolls feel like a camera whip-pan with
       motion-chroma fringing that settles back to a calm iris
       split when you stop. */
    float chromaAmt = 0.04 + vel * 0.14;
    float nr = smoothstep(0.05, 0.95, f2.z + chromaAmt);
    float nb = smoothstep(0.05, 0.95, f2.z - chromaAmt);

    /* Wide gentle radial falloff — atmosphere, not vignette. */
    float rr = length(p * vec2(0.85, 1.05));
    float falloff = smoothstep(2.00, 0.05, rr);

    /* ---- DARK mode path (screen blend target) ----
       HDR lit goes through ACES tonemapping so highlights keep
       their chroma instead of blowing to white. Screen blend then
       adds the tonemapped colour over the page bg. */
    vec3 darkHdr = lit * 0.85;
    darkHdr.r *= 1.0 + 0.22 * nr;
    darkHdr.b *= 1.0 + 0.22 * nb;
    vec3 darkOut = aces(darkHdr) * 0.65;
    darkOut *= mix(0.45, 1.0, falloff);

    /* ---- LIGHT mode path (multiply blend target) ----
       Physically-based GREY smoke on paper. No iris colour wash.
       Real smoke scatters sunlight forward (HG phase function),
       absorbs light via Beer-Lambert along a shadow ray toward the
       sun, and picks up a blue-ish ambient fill from the sky.

       Lighting model per pixel:
         L(x) = albedo * density * (sun * exp(-shadow) * phase
                                 + sky * ambientOcclusion)
         alpha = 1 - exp(-density * extinction)

       Output for multiply-blend: mix white paper with the lit smoke
       colour by the opacity, so dense regions darken & tint while
       the paper stays paper where the smoke is thin. */

    // Real smoke optical properties — slightly blueish albedo
    // (sub-micron soot scatters blue a touch more), single-
    // scattering albedo near 0.9 is typical for fresh smoke.
    vec3  smokeAlbedo  = vec3(0.93, 0.94, 0.96);
    vec3  sunSpectrum  = vec3(1.00, 0.97, 0.90);  // warm direct
    vec3  skyAmbient   = vec3(0.70, 0.78, 0.95);  // cool diffuse
    float smokeExtL    = 2.4;  // extinction coefficient for alpha

    // Raw density from the 3 parallax layers (no smoothstep shaping
    // here — keep the density field close to the actual volume).
    float rawDensity = clamp(0.5 * d0 + 0.8 * d1 + 1.0 * d2, 0.0, 1.5)
                     * mood.intensity;

    // IGN-jittered shadow ray: march toward the sun and accumulate
    // density so pixels "behind" thick smoke are lit dimmer than
    // pixels at the front face. Blue-noise start offset prevents
    // radial banding in the shadow gradient.
    vec2 sunDirL = rayDir;
    float shadowD = 0.0;
    const int SHADOW_STEPS = 5;
    float sStep = 0.15;
    float jitterL = ign(gl_FragCoord.xy + vec2(uTime * 37.0, 11.0));
    for (int i = 0; i < SHADOW_STEPS; i++) {
      float ti = (float(i) + jitterL) * sStep;
      vec2 sp = p + sunDirL * ti;
      vec3 sf = smokeField(sp * 1.1 * mood.scale + mood.off, t + idx * 0.35);
      shadowD += sf.x;
    }
    shadowD /= float(SHADOW_STEPS);

    // Direct sunlight through the smoke. Intentionally NOT using a
    // phase function here - in a 2D shader the view-sun alignment
    // direction is deterministic per pixel, so any forward-scatter
    // phase produces a visible beam as the sun drifts. Real cloud
    // silver lining is a 3D shape effect that we can't faithfully
    // fake at this dimensionality; better to drop it than to ship a
    // stray beam. What remains is physically correct: Beer-Lambert
    // attenuation along the shadow ray toward the sun, tinted by
    // the solar spectrum.
    vec3 direct = sunSpectrum * exp(-shadowD * 3.2);

    // Ambient sky fill, attenuated by the local density so the
    // underside of thick clouds stays darker (classic cloudscape).
    float ambOcc = exp(-rawDensity * 0.5);
    vec3 ambient = skyAmbient * ambOcc * 0.75;

    // Scattered radiance out of the smoke toward the eye.
    vec3 smokeL = smokeAlbedo * rawDensity * (direct + ambient);

    // Opacity via Beer-Lambert on total extinction along view depth.
    float alphaL = 1.0 - exp(-rawDensity * smokeExtL);
    alphaL *= mix(0.55, 1.0, falloff);

    // Compose: where smoke is thin, paper stays paper; where thick,
    // it takes on the grey-blue lit smoke colour.
    vec3 lightOut = mix(vec3(1.0), smokeL, alphaL);

    vec3 col = mix(lightOut, darkOut, uTheme);

    /* Cinema vignette — soft darkening at the corners, only in
       dark mode (light mode already feels bright enough). */
    float vignette = 1.0 - smoothstep(0.6, 1.35, length(p * vec2(0.9, 1.0)));
    col *= mix(1.0, mix(0.85, 1.0, vignette), uTheme);

    /* Jimenez 2014 Interleaved Gradient Noise grain — low-
       discrepancy, temporally stable sequence originally developed
       for Call of Duty's Temporal Anti-Aliasing. Unlike hash-based
       noise it has clean blue-noise-like spectrum, so banding is
       killed without the "crawling ants" look hash dither produces. */
    float gn = ign(gl_FragCoord.xy + vec2(uTime * 60.0, uTime * 37.0));
    float grain = (gn - 0.5) * mix(0.010, 0.028, uTheme);
    col += grain;

    gl_FragColor = vec4(col, 1.0);
  }
`;
