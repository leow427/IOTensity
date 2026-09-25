import {
  Canvas,
  useFrame,
  useThree,
  type ThreeEvent,
} from '@react-three/fiber';
import {
  Component,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import {
  Color,
  Mesh,
  MeshBasicMaterial,
  OrthographicCamera,
  Plane,
  Vector3,
} from 'three';
import { OrbitControls } from 'three/addons/controls/OrbitControls.js';
import { resolveColor } from '../domain/colors';
import type { EditMode, Room, VirtualLight } from '../domain/model';
import { useStore } from '../state/context';
import type { AppStore } from '../state/store';

function Box({
  at,
  size,
  color,
  rotation = 0,
}: {
  at: [number, number, number];
  size: [number, number, number];
  color: string;
  rotation?: number;
}) {
  return (
    <mesh position={at} rotation={[0, rotation, 0]}>
      <boxGeometry args={size} />
      <meshStandardMaterial color={color} roughness={0.82} />
    </mesh>
  );
}

function Furniture() {
  return (
    <group>
      <Box at={[0, -0.08, 1.55]} size={[6.45, 0.16, 5.2]} color="#566058" />
      <Box at={[0, 1.5, -1.05]} size={[6.45, 3, 0.12]} color="#636e63" />
      <Box at={[-3.25, 1.5, 1.55]} size={[0.12, 3, 5.2]} color="#505c52" />
      <Box at={[0, 0.08, -0.96]} size={[6.3, 0.14, 0.05]} color="#404940" />
      <Box at={[-3.16, 0.08, 1.55]} size={[0.05, 0.14, 5]} color="#404940" />
      <gridHelper
        args={[6, 12, '#707b6e', '#646e61']}
        position={[0, 0.005, 1.5]}
        scale={[1, 1, 0.82]}
      />
      {/* Desk, drawers, work mat and keyboard. Monitor centre projects to the origin. */}
      <Box at={[0, 0.84, 0.08]} size={[3.2, 0.13, 1.25]} color="#b8b3a2" />
      <Box at={[-1.13, 0.4, 0.08]} size={[0.65, 0.8, 1.08]} color="#999889" />
      <Box at={[1.23, 0.4, 0.08]} size={[0.12, 0.8, 1.05]} color="#949685" />
      {[0.23, 0.48, 0.7].map((y) => (
        <Box
          key={y}
          at={[-1.13, y, 0.628]}
          size={[0.24, 0.025, 0.015]}
          color="#545b51"
        />
      ))}
      <Box at={[0.2, 0.913, 0.25]} size={[1.55, 0.01, 0.52]} color="#5b6257" />
      <Box
        at={[-0.04, 0.934, 0.26]}
        size={[0.7, 0.035, 0.24]}
        color="#a4aa98"
      />
      <Box at={[0.66, 0.94, 0.29]} size={[0.12, 0.055, 0.19]} color="#a4aa98" />
      <Box at={[0, 0.955, -0.04]} size={[0.44, 0.07, 0.3]} color="#292e29" />
      <Box at={[0, 1.16, -0.09]} size={[0.095, 0.4, 0.07]} color="#30352f" />
      <Box at={[0, 1.58, 0]} size={[1.75, 1.04, 0.1]} color="#252b27" />
      <mesh position={[0, 1.59, 0.057]}>
        <planeGeometry args={[1.63, 0.89]} />
        <meshBasicMaterial color="#657c70" />
      </mesh>
      <mesh position={[-0.43, 1.59, 0.061]}>
        <planeGeometry args={[0.77, 0.89]} />
        <meshBasicMaterial color="#879588" />
      </mesh>
      <mesh position={[0.23, 1.77, 0.064]}>
        <planeGeometry args={[0.37, 0.27]} />
        <meshBasicMaterial color="#c4bba0" />
      </mesh>
      <mesh position={[0.47, 1.38, 0.064]}>
        <planeGeometry args={[0.66, 0.43]} />
        <meshBasicMaterial color="#527b6e" />
      </mesh>
      {/* A compact office chair with five-spoke base. */}
      <group position={[0.1, 0, 1.6]} rotation={[0, -0.22, 0]}>
        <Box at={[0, 0.55, 0]} size={[0.7, 0.12, 0.68]} color="#333b36" />
        <Box at={[0, 1.02, 0.27]} size={[0.67, 0.83, 0.12]} color="#39433c" />
        <Box at={[0, 0.28, 0]} size={[0.075, 0.48, 0.075]} color="#252c28" />
        {[-1, 1].map((sign) => (
          <group key={sign}>
            <Box
              at={[sign * 0.42, 0.8, 0]}
              size={[0.09, 0.07, 0.46]}
              color="#2c352e"
            />
            <Box
              at={[sign * 0.4, 0.65, 0.1]}
              size={[0.05, 0.3, 0.05]}
              color="#2c352e"
            />
          </group>
        ))}
        {[0, 1, 2, 3, 4].map((n) => (
          <group key={n} rotation={[0, (n * Math.PI * 2) / 5, 0]}>
            <Box
              at={[0, 0.1, 0.22]}
              size={[0.065, 0.055, 0.52]}
              color="#303a32"
            />
            <mesh position={[0, 0.08, 0.43]}>
              <sphereGeometry args={[0.065, 10, 8]} />
              <meshStandardMaterial color="#222b25" />
            </mesh>
          </group>
        ))}
      </group>
      {/* Plant-like geometry adds scale without external assets. */}
      <mesh position={[-2.62, 0.25, -0.45]}>
        <cylinderGeometry args={[0.23, 0.17, 0.5, 16]} />
        <meshStandardMaterial color="#a5997f" />
      </mesh>
      <mesh position={[-2.62, 0.75, -0.45]} scale={[0.35, 0.6, 0.35]}>
        <icosahedronGeometry args={[1, 1]} />
        <meshStandardMaterial color="#788b6b" />
      </mesh>
    </group>
  );
}

function CameraControl({
  dragging,
  resetKey,
}: {
  dragging: boolean;
  resetKey: number;
}) {
  const { camera, gl, size } = useThree();
  const controls = useMemo(() => new OrbitControls(camera), [camera]);
  useEffect(() => {
    // Bind listeners in the effect so React's development remounts reconnect
    // cleanly, without side effects from a discarded render.
    controls.connect(gl.domElement);
    controls.target.set(0, 0.9, 1.15);
    controls.enablePan = false;
    controls.enableDamping = true;
    controls.dampingFactor = 0.12;
    controls.minPolarAngle = Math.PI / 5;
    controls.maxPolarAngle = Math.PI / 2.6;
    controls.minAzimuthAngle = -Math.PI / 5;
    controls.maxAzimuthAngle = Math.PI / 2.5;
    controls.minZoom = 34;
    controls.maxZoom = 130;
    return () => controls.dispose();
  }, [controls, gl]);
  useEffect(() => {
    controls.enabled = !dragging;
  }, [controls, dragging]);
  useEffect(() => {
    camera.position.set(7.4, 6.8, 9);
    if (camera instanceof OrthographicCamera) {
      camera.zoom = Math.min(size.width / 9.5, size.height / 6.8);
      camera.updateProjectionMatrix();
    }
    controls.target.set(0, 1, 1.3);
    controls.update();
  }, [camera, controls, resetKey, size.width, size.height]);
  useFrame(() => controls.update());
  return null;
}

function Orb({
  light,
  selected,
  mode,
  editable,
  store,
  setDragging,
}: {
  light: VirtualLight;
  selected: boolean;
  mode: EditMode;
  editable: boolean;
  store: AppStore;
  setDragging: (value: boolean) => void;
}) {
  const orb = useRef<Mesh>(null);
  const halo = useRef<Mesh>(null);
  const selectionRing = useRef<Mesh>(null);
  const material = useRef<MeshBasicMaterial>(null);
  const haloMaterial = useRef<MeshBasicMaterial>(null);
  const { camera, gl } = useThree();
  const drag = useRef<{
    plane: Plane;
    offset: Vector3;
    pointerId: number;
    x: number;
    y: number;
    started: boolean;
  } | null>(null);
  const point = useMemo(() => new Vector3(), []);
  const color = useMemo(() => new Color(), []);
  const p = light.position;

  useFrame(() => {
    const rgb = resolveColor(
      light,
      store.output.getColor(light.id),
      selected && editable ? mode : undefined,
    );
    color.setRGB(...rgb, 'srgb');
    material.current?.color.copy(color);
    haloMaterial.current?.color.copy(color);
    if (halo.current) halo.current.quaternion.copy(camera.quaternion);
    if (selectionRing.current)
      selectionRing.current.quaternion.copy(camera.quaternion);
  });
  useEffect(
    () => () => {
      if (drag.current) setDragging(false);
      gl.domElement.style.cursor = '';
    },
    [gl, setDragging],
  );

  const down = (event: ThreeEvent<PointerEvent>) => {
    if (!editable || !store.canEdit || event.button !== 0) return;
    event.stopPropagation();
    store.selectLight(light.id);
    const normal =
      mode === 'location'
        ? new Vector3(0, 1, 0)
        : camera.getWorldDirection(new Vector3()).setY(0).normalize();
    const plane = new Plane().setFromNormalAndCoplanarPoint(
      normal,
      new Vector3(p.x, p.y, p.z),
    );
    event.ray.intersectPlane(plane, point);
    drag.current = {
      plane,
      offset: new Vector3(p.x, p.y, p.z).sub(point),
      pointerId: event.pointerId,
      x: event.clientX,
      y: event.clientY,
      started: false,
    };
    (event.target as Element).setPointerCapture(event.pointerId);
    setDragging(true);
    gl.domElement.style.cursor = 'grabbing';
  };
  const move = (event: ThreeEvent<PointerEvent>) => {
    if (!drag.current) return;
    event.stopPropagation();
    if (
      Math.hypot(
        event.clientX - drag.current.x,
        event.clientY - drag.current.y,
      ) > 3
    )
      drag.current.started = true;
    if (
      drag.current.started &&
      event.ray.intersectPlane(drag.current.plane, point)
    ) {
      point.add(drag.current.offset);
      store.moveLight(light.id, { x: point.x, y: point.y, z: point.z });
    }
  };
  const up = (event: ThreeEvent<PointerEvent>) => {
    if (!drag.current) return;
    event.stopPropagation();
    (event.target as Element).releasePointerCapture(drag.current.pointerId);
    drag.current = null;
    setDragging(false);
    gl.domElement.style.cursor = 'grab';
  };

  return (
    <group position={[p.x, 0, p.z]}>
      <mesh position={[0, p.y / 2, 0]}>
        <cylinderGeometry args={[0.008, 0.008, p.y, 6]} />
        <meshBasicMaterial
          color={selected ? '#ecebe0' : '#a3b1a0'}
          transparent
          opacity={selected ? 0.85 : 0.44}
        />
      </mesh>
      <mesh position={[0, 0.015, 0]} rotation={[-Math.PI / 2, 0, 0]}>
        <ringGeometry args={[0.065, 0.09, 24]} />
        <meshBasicMaterial color="#c3cdbc" transparent opacity={0.6} />
      </mesh>
      <mesh ref={halo} position={[0, p.y, 0]}>
        <circleGeometry args={[0.27, 32]} />
        <meshBasicMaterial
          ref={haloMaterial}
          toneMapped={false}
          transparent
          opacity={0.09}
          depthWrite={false}
        />
      </mesh>
      <mesh
        ref={orb}
        position={[0, p.y, 0]}
        onPointerDown={down}
        onPointerMove={move}
        onPointerUp={up}
        onPointerCancel={up}
        onLostPointerCapture={() => {
          drag.current = null;
          setDragging(false);
        }}
        onPointerOver={() => {
          if (editable) gl.domElement.style.cursor = 'grab';
        }}
        onPointerOut={() => {
          if (!drag.current) gl.domElement.style.cursor = '';
        }}
      >
        <sphereGeometry args={[0.12, 24, 20]} />
        <meshBasicMaterial ref={material} toneMapped={false} />
      </mesh>
      {selected && editable && (
        <mesh
          ref={selectionRing}
          position={[0, p.y, 0]}
          quaternion={camera.quaternion}
        >
          <ringGeometry args={[0.175, 0.19, 40]} />
          <meshBasicMaterial color="#f7f3de" depthTest={false} />
        </mesh>
      )}
    </group>
  );
}

class SceneBoundary extends Component<
  { children: ReactNode },
  { failed: boolean }
> {
  state = { failed: false };
  static getDerivedStateFromError() {
    return { failed: true };
  }
  render() {
    return this.state.failed ? (
      <div className="scene-fallback">
        <p>3D view unavailable</p>
        <span>Use the light cards and position controls.</span>
      </div>
    ) : (
      this.props.children
    );
  }
}

export function RoomScene({
  room,
  selectedId = null,
  mode = 'location',
  editable = false,
  resetKey = 0,
}: {
  room: Room;
  selectedId?: string | null;
  mode?: EditMode;
  editable?: boolean;
  resetKey?: number;
}) {
  const store = useStore();
  const [dragging, setDragging] = useState(false);
  return (
    <SceneBoundary>
      <Canvas
        role="img"
        orthographic
        camera={{ position: [7.4, 6.8, 9], zoom: 64, near: 0.1, far: 80 }}
        dpr={[1, 1.7]}
        gl={{ antialias: true, alpha: true }}
        onPointerMissed={(event) => {
          if (editable && event.type === 'click') store.selectLight(null);
        }}
        aria-label={
          editable
            ? 'Interactive 3D room. Use light cards and coordinate controls for keyboard access.'
            : 'Saved room preview'
        }
      >
        <ambientLight intensity={1.5} />
        <directionalLight
          position={[2, 7, 5]}
          intensity={2.4}
          color="#faf5dd"
        />
        <Furniture />
        {room.lights.map((light) => (
          <Orb
            key={light.id}
            light={light}
            selected={selectedId === light.id}
            mode={mode}
            editable={editable}
            store={store}
            setDragging={setDragging}
          />
        ))}
        <CameraControl dragging={dragging} resetKey={resetKey} />
      </Canvas>
    </SceneBoundary>
  );
}
