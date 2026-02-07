import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";

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

// --- Animation loop ---
function animate(): void {
  controls.update();
  renderer.render(scene, camera);
}
renderer.setAnimationLoop(animate);

// --- Responsive resize ---
window.addEventListener("resize", () => {
  camera.aspect = window.innerWidth / window.innerHeight;
  camera.updateProjectionMatrix();
  renderer.setSize(window.innerWidth, window.innerHeight);
});
