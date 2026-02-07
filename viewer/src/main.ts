import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import {
  parseOrbitCSV,
  createOrbitVisualization,
  updateSatellitePosition,
  updateOrbitTrail,
  OrbitVisualization,
  OrbitPoint,
} from "./orbit.js";
import { PlaybackController } from "./playback.js";

// --- Scene setup ---
const scene = new THREE.Scene();
scene.background = new THREE.Color(0x000000);

// --- Camera ---
// Position camera so that Earth (radius=1) is visible with some margin
const camera = new THREE.PerspectiveCamera(
  60,
  window.innerWidth / window.innerHeight,
  0.01,
  1000
);
camera.position.set(0, 2, 5);

// --- Renderer ---
const renderer = new THREE.WebGLRenderer({ antialias: true });
renderer.setSize(window.innerWidth, window.innerHeight);
renderer.setPixelRatio(window.devicePixelRatio);
document.body.appendChild(renderer.domElement);

// --- OrbitControls ---
const controls = new OrbitControls(camera, renderer.domElement);
controls.enableDamping = true;
controls.dampingFactor = 0.1;
controls.minDistance = 1.5;
controls.maxDistance = 100;

// --- Lighting ---
// Ambient light for general illumination
const ambientLight = new THREE.AmbientLight(0x404040, 1.0);
scene.add(ambientLight);

// Directional light simulating sunlight
const directionalLight = new THREE.DirectionalLight(0xffffff, 2.0);
directionalLight.position.set(5, 3, 5);
scene.add(directionalLight);

// --- Earth ---
// Radius = 1 unit (represents 6378 km in scene scale)
const earthGeometry = new THREE.SphereGeometry(1, 64, 64);
const earthMaterial = new THREE.MeshPhongMaterial({
  color: 0x2255aa,
  emissive: 0x112244,
  emissiveIntensity: 0.1,
  shininess: 25,
});
const earth = new THREE.Mesh(earthGeometry, earthMaterial);
scene.add(earth);

// --- Wireframe overlay for reference grid ---
const wireframeGeometry = new THREE.SphereGeometry(1.002, 24, 24);
const wireframeMaterial = new THREE.MeshBasicMaterial({
  color: 0x4488cc,
  wireframe: true,
  transparent: true,
  opacity: 0.15,
});
const wireframe = new THREE.Mesh(wireframeGeometry, wireframeMaterial);
scene.add(wireframe);

// --- Axes helper (X=red, Y=green, Z=blue, length = 2 Earth radii) ---
const axesHelper = new THREE.AxesHelper(2);
scene.add(axesHelper);

// --- Orbit visualization state ---
let currentOrbit: OrbitVisualization | null = null;
let currentPoints: OrbitPoint[] = [];

function clearOrbit(): void {
  if (currentOrbit) {
    scene.remove(currentOrbit.orbitLine);
    scene.remove(currentOrbit.satelliteMarker);
    currentOrbit.orbitLine.geometry.dispose();
    (currentOrbit.orbitLine.material as THREE.Material).dispose();
    currentOrbit.satelliteMarker.geometry.dispose();
    (currentOrbit.satelliteMarker.material as THREE.Material).dispose();
    currentOrbit = null;
  }
  currentPoints = [];
}

// --- Playback state ---
let playback: PlaybackController | null = null;

// --- Playback UI elements ---
const playbackBar = document.getElementById("playback-bar") as HTMLDivElement;
const playPauseBtn = document.getElementById(
  "play-pause-btn"
) as HTMLButtonElement;
const speedSelect = document.getElementById(
  "speed-select"
) as HTMLSelectElement;
const timeSlider = document.getElementById("time-slider") as HTMLInputElement;
const timeDisplay = document.getElementById("time-display") as HTMLSpanElement;

// --- CSV file loading ---
const loadBtn = document.getElementById("load-csv-btn") as HTMLButtonElement;
const fileInput = document.getElementById(
  "csv-file-input"
) as HTMLInputElement;
const orbitInfo = document.getElementById("orbit-info") as HTMLDivElement;

loadBtn.addEventListener("click", () => {
  fileInput.click();
});

fileInput.addEventListener("change", () => {
  const file = fileInput.files?.[0];
  if (!file) return;

  const reader = new FileReader();
  reader.onload = () => {
    const text = reader.result as string;
    const points = parseOrbitCSV(text);

    if (points.length === 0) {
      orbitInfo.style.display = "block";
      orbitInfo.textContent = "No valid orbit data found in file.";
      return;
    }

    // Remove previous orbit
    clearOrbit();

    // Store points for playback
    currentPoints = points;

    // Create and add new orbit visualization
    currentOrbit = createOrbitVisualization(points);
    scene.add(currentOrbit.orbitLine);
    scene.add(currentOrbit.satelliteMarker);

    // Show orbit info
    const duration = points[points.length - 1].t - points[0].t;
    orbitInfo.style.display = "block";
    orbitInfo.textContent =
      `Loaded: ${file.name} | ${points.length} points | ` +
      `Duration: ${duration.toFixed(1)} s`;

    // Initialize playback controller
    playback = new PlaybackController(points);
    playback.onChange = syncPlaybackUI;

    // Show playback bar and set initial state
    playbackBar.classList.add("visible");
    syncPlaybackUI();

    // Move satellite to starting position and hide full trail
    if (currentOrbit) {
      updateSatellitePosition(
        currentOrbit.satelliteMarker,
        playback.getCurrentState()
      );
      updateOrbitTrail(currentOrbit.orbitLine, 1, currentPoints.length);
    }
  };

  reader.readAsText(file);

  // Reset file input so the same file can be re-loaded
  fileInput.value = "";
});

// --- Playback UI wiring ---

playPauseBtn.addEventListener("click", () => {
  if (playback) {
    playback.togglePlayPause();
  }
});

speedSelect.addEventListener("change", () => {
  if (playback) {
    playback.setSpeed(Number(speedSelect.value));
  }
});

let isScrubbing = false;

timeSlider.addEventListener("input", () => {
  if (playback) {
    isScrubbing = true;
    const fraction = Number(timeSlider.value) / 1000;
    playback.seekToFraction(fraction);
  }
});

timeSlider.addEventListener("change", () => {
  isScrubbing = false;
});

/**
 * Format a time value in seconds to a human-readable string.
 * Shows minutes and seconds when >= 60s, otherwise just seconds.
 */
function formatTime(seconds: number): string {
  if (seconds < 60) {
    return `${seconds.toFixed(1)} s`;
  }
  const mins = Math.floor(seconds / 60);
  const secs = seconds % 60;
  return `${mins}m ${secs.toFixed(1)}s`;
}

/**
 * Synchronise all playback UI elements with the current controller state.
 */
function syncPlaybackUI(): void {
  if (!playback) return;

  // Play/Pause button label
  playPauseBtn.textContent = playback.isPlaying ? "Pause" : "Play";

  // Time slider (update only when user is not dragging)
  if (!isScrubbing) {
    timeSlider.value = String(Math.round(playback.fraction * 1000));
  }

  // Time display
  timeDisplay.textContent =
    `T+${formatTime(playback.elapsedTime)} / ${formatTime(playback.totalDuration)}`;
}

// --- Animation loop ---
const clock = new THREE.Clock();

function animate(): void {
  const dt = clock.getDelta();
  controls.update();

  // Advance playback if active
  if (playback && currentOrbit) {
    const changed = playback.update(dt);
    if (changed || isScrubbing) {
      // Update satellite position via interpolation
      const state = playback.getCurrentState();
      updateSatellitePosition(currentOrbit.satelliteMarker, state);

      // Update visible trail: show points up to current time + 1
      // (+1 so at least the first vertex is always shown)
      const trailIndex = playback.getCurrentTrailIndex();
      updateOrbitTrail(
        currentOrbit.orbitLine,
        trailIndex + 2, // +2: index is 0-based, and we want one past
        currentPoints.length
      );

      // Sync UI every frame during active playback
      syncPlaybackUI();
    }
  }

  renderer.render(scene, camera);
}
renderer.setAnimationLoop(animate);

// --- Responsive resize ---
window.addEventListener("resize", () => {
  camera.aspect = window.innerWidth / window.innerHeight;
  camera.updateProjectionMatrix();
  renderer.setSize(window.innerWidth, window.innerHeight);
});
