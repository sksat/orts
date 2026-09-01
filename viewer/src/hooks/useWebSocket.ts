import { useCallback, useEffect, useRef, useState } from "react";
import type { OrbitPoint } from "../orbit.js";
import type { AttitudePayload } from "../protocol/generated/AttitudePayload.js";
import type { ClientMessage } from "../protocol/generated/ClientMessage.js";
import type { HistoryState } from "../protocol/generated/HistoryState.js";
import type { SatelliteInfo as WireSatelliteInfo } from "../protocol/generated/SatelliteInfo.js";
import type { WsMessage } from "../protocol/generated/WsMessage.js";
import type { MarkerShape } from "../satelliteShapes.js";
import { describeCentralBodyError, resolveCentralBody } from "../sources/centralBody.js";

/** Per-satellite info from the server, normalized for app use. */
export interface SatelliteInfo {
  id: string;
  name: string | null;
  altitude: number;
  period: number;
  /** Names of active perturbation force models (e.g. "drag", "srp"). */
  perturbations: string[];
  /** Sim-declared viewer marker shape, or null when unset (viewer decides). */
  shape: MarkerShape | null;
}

/**
 * Simulation metadata sent by the server on initial connection,
 * normalized for app use (absent fields resolved to defaults).
 *
 * Corresponds to the `{"type":"info",...}` message from
 * `orts serve`.
 */
export interface SimInfo {
  mu: number;
  dt: number;
  output_interval: number;
  stream_interval: number;
  central_body: string;
  central_body_radius: number;
  /** Julian Date of the simulation epoch, or null if not set. */
  epoch_jd: number | null;
  /** List of satellites in the simulation. */
  satellites: SatelliteInfo[];
}

/**
 * Raw server→client wire message.
 *
 * Generated from the Rust `WsMessage` enum
 * (`cli/src/commands/serve/protocol.rs`) by ts-rs; regenerate with
 * `cargo test -p orts-cli`. The dispatch below still applies runtime
 * fallbacks for fields that older servers omitted.
 */
export type ServerMessage = WsMessage;

/** Response data from a query_range request. */
export interface QueryRangeResponse {
  tMin: number;
  tMax: number;
  points: OrbitPoint[];
}

export interface UseWebSocketOptions {
  /** WebSocket server URL, e.g. "ws://localhost:9001". */
  url: string;
  /** Called for each orbit state update received from the server. */
  onState: (state: OrbitPoint) => void;
  /** Called when the server sends simulation metadata (on connect). */
  onInfo: (info: SimInfo) => void;
  onHistory: (points: OrbitPoint[]) => void;
  /** Called when the server responds to a query_range request. */
  onQueryRangeResponse?: (response: QueryRangeResponse) => void;
  /** Called when a satellite's simulation terminates (collision, atmospheric entry, etc.). */
  onSimulationTerminated?: (entityPath: string, t: number, reason: string) => void;
  /** Called when the server sends its status (e.g. "idle"). */
  onStatus?: (state: string) => void;
  /** Called when the server sends an error message. */
  onError?: (message: string) => void;
  /** Called when the server notifies that high-res textures are available for a body. */
  onTexturesReady?: (body: string) => void;
  /** Called when the server confirms a satellite was added to a running simulation. */
  onSatelliteAdded?: (satellite: SatelliteInfo, t: number) => void;
}

/** Callbacks for message dispatch (subset of UseWebSocketOptions used by dispatchServerMessage). */
export interface DispatchCallbacks {
  onState: (state: OrbitPoint) => void;
  onInfo?: (info: SimInfo) => void;
  onHistory?: (points: OrbitPoint[]) => void;
  onQueryRangeResponse?: (response: QueryRangeResponse) => void;
  onSimulationTerminated?: (entityPath: string, t: number, reason: string) => void;
  onStatus?: (state: string) => void;
  onError?: (message: string) => void;
  onTexturesReady?: (body: string) => void;
  onSatelliteAdded?: (satellite: SatelliteInfo, t: number) => void;
}

/** Normalize a wire SatelliteInfo (absent fields) into the app-level shape. */
function normalizeSatelliteInfo(s: WireSatelliteInfo): SatelliteInfo {
  return {
    id: s.id,
    name: s.name ?? null,
    altitude: s.altitude,
    period: s.period,
    perturbations: s.perturbations ?? [],
    shape: s.shape ?? null,
  };
}

function parseAccelerations(accels?: Record<string, number>) {
  return {
    accel_gravity: accels?.gravity ?? 0,
    accel_drag: accels?.drag ?? 0,
    accel_srp: accels?.srp ?? 0,
    accel_third_body_sun: accels?.third_body_sun ?? 0,
    accel_third_body_moon: accels?.third_body_moon ?? 0,
  };
}

function parseAttitude(attitude?: AttitudePayload) {
  if (!attitude) return {};
  const [qw, qx, qy, qz] = attitude.quaternion_wxyz;
  const [wx, wy, wz] = attitude.angular_velocity_body;
  return { qw, qx, qy, qz, wx, wy, wz };
}

function parseHistoryPoints(states: HistoryState[]): OrbitPoint[] {
  return states.map((s) => ({
    entityPath: s.entity_path,
    t: s.t,
    x: s.position[0],
    y: s.position[1],
    z: s.position[2],
    vx: s.velocity[0],
    vy: s.velocity[1],
    vz: s.velocity[2],
    a: s.semi_major_axis,
    e: s.eccentricity,
    inc: s.inclination,
    raan: s.raan,
    omega: s.argument_of_periapsis,
    nu: s.true_anomaly,
    altitude: s.altitude,
    specific_energy: s.specific_energy,
    angular_momentum: s.angular_momentum,
    velocity_mag: s.velocity_mag,
    ...parseAccelerations(s.accelerations),
    ...parseAttitude(s.attitude),
  }));
}

/**
 * Dispatch a parsed server message to the appropriate callback.
 * Extracted as a pure function for testability.
 */
export function dispatchServerMessage(msg: ServerMessage, callbacks: DispatchCallbacks): void {
  if (msg.type === "state") {
    callbacks.onState({
      entityPath: msg.entity_path,
      t: msg.t,
      x: msg.position[0],
      y: msg.position[1],
      z: msg.position[2],
      vx: msg.velocity[0],
      vy: msg.velocity[1],
      vz: msg.velocity[2],
      a: msg.semi_major_axis,
      e: msg.eccentricity,
      inc: msg.inclination,
      raan: msg.raan,
      omega: msg.argument_of_periapsis,
      nu: msg.true_anomaly,
      altitude: msg.altitude,
      specific_energy: msg.specific_energy,
      angular_momentum: msg.angular_momentum,
      velocity_mag: msg.velocity_mag,
      ...parseAccelerations(msg.accelerations),
      ...parseAttitude(msg.attitude),
    });
  } else if (msg.type === "info") {
    // A server that predates `central_body_radius` leaves it out, and the body
    // it names says which radius that is. Resolved the way a file's is, so a
    // live source cannot be read against a body a file would be refused for.
    const resolved = resolveCentralBody({
      bodyId: msg.central_body,
      mu: msg.mu,
      bodyRadius: msg.central_body_radius,
    });
    if (!resolved.ok) {
      callbacks.onError?.(
        `the server's simulation info: ${describeCentralBodyError(resolved.error)}`,
      );
      return;
    }
    // The `??` fallbacks tolerate older servers that predate these fields.
    const satellites: SatelliteInfo[] = (msg.satellites ?? []).map(normalizeSatelliteInfo);
    callbacks.onInfo?.({
      mu: resolved.body.mu,
      dt: msg.dt,
      output_interval: msg.output_interval,
      stream_interval: msg.stream_interval ?? msg.output_interval,
      central_body: resolved.body.bodyId,
      central_body_radius: resolved.body.bodyRadius,
      epoch_jd: msg.epoch_jd ?? null,
      satellites,
    });
  } else if (msg.type === "history") {
    callbacks.onHistory?.(parseHistoryPoints(msg.states));
  } else if (msg.type === "query_range_response") {
    callbacks.onQueryRangeResponse?.({
      tMin: msg.t_min,
      tMax: msg.t_max,
      points: parseHistoryPoints(msg.states),
    });
  } else if (msg.type === "simulation_terminated") {
    callbacks.onSimulationTerminated?.(msg.entity_path, msg.t, msg.reason);
  } else if (msg.type === "status") {
    callbacks.onStatus?.(msg.state);
  } else if (msg.type === "error") {
    callbacks.onError?.(msg.message);
  } else if (msg.type === "textures_ready") {
    callbacks.onTexturesReady?.(msg.body);
  } else if (msg.type === "satellite_added") {
    callbacks.onSatelliteAdded?.(normalizeSatelliteInfo(msg.satellite), msg.t);
  }
}

export interface UseWebSocketReturn {
  /** Open a WebSocket connection to the configured URL. */
  connect: () => void;
  /** Close the active WebSocket connection. */
  disconnect: () => void;
  /** Whether a WebSocket connection is currently open. */
  isConnected: boolean;
  /** Send a client→server message (typed against the generated wire contract). */
  send: (msg: ClientMessage) => void;
}

/**
 * React hook for connecting to the orts simulation WebSocket server.
 *
 * Manages the WebSocket lifecycle (connect/disconnect), parses incoming
 * JSON messages, and dispatches them to the appropriate callbacks.
 *
 * The connection is automatically cleaned up when the component unmounts.
 */
export function useWebSocket(options: UseWebSocketOptions): UseWebSocketReturn {
  const [isConnected, setIsConnected] = useState(false);
  const wsRef = useRef<WebSocket | null>(null);

  // Keep callbacks in refs so we don't need to reconnect when they change.
  const callbacksRef = useRef<DispatchCallbacks>({
    onState: options.onState,
    onInfo: options.onInfo,
    onHistory: options.onHistory,
    onQueryRangeResponse: options.onQueryRangeResponse,
    onSimulationTerminated: options.onSimulationTerminated,
    onStatus: options.onStatus,
    onError: options.onError,
    onTexturesReady: options.onTexturesReady,
    onSatelliteAdded: options.onSatelliteAdded,
  });
  callbacksRef.current = {
    onState: options.onState,
    onInfo: options.onInfo,
    onHistory: options.onHistory,
    onQueryRangeResponse: options.onQueryRangeResponse,
    onSimulationTerminated: options.onSimulationTerminated,
    onStatus: options.onStatus,
    onError: options.onError,
    onTexturesReady: options.onTexturesReady,
    onSatelliteAdded: options.onSatelliteAdded,
  };

  const urlRef = useRef(options.url);
  urlRef.current = options.url;

  const disconnect = useCallback(() => {
    if (wsRef.current) {
      wsRef.current.close();
      wsRef.current = null;
    }
    setIsConnected(false);
  }, []);

  const connect = useCallback(() => {
    // Close any existing connection first
    if (wsRef.current) {
      wsRef.current.close();
      wsRef.current = null;
    }

    const ws = new WebSocket(urlRef.current);
    wsRef.current = ws;

    ws.addEventListener("open", () => {
      // Only set connected if this is still the active WebSocket.
      // If connect() was called again, wsRef.current points to the new one.
      if (wsRef.current === ws) {
        setIsConnected(true);
      }
    });

    ws.addEventListener("close", () => {
      // Only reset state if this is still the active WebSocket.
      // A stale close handler from a previous connection must not
      // corrupt the new connection's state.
      if (wsRef.current === ws) {
        setIsConnected(false);
        wsRef.current = null;
      }
    });

    ws.addEventListener("error", () => {
      // The close event will fire after error, which resets state.
      // Nothing extra to do here.
    });

    ws.addEventListener("message", (event: MessageEvent) => {
      try {
        const msg = JSON.parse(event.data as string) as ServerMessage;
        dispatchServerMessage(msg, callbacksRef.current);
      } catch {
        // Silently ignore malformed messages
      }
    });
  }, []);

  const send = useCallback((msg: ClientMessage) => {
    if (wsRef.current?.readyState === WebSocket.OPEN) {
      wsRef.current.send(JSON.stringify(msg));
    }
  }, []);

  // Clean up on unmount
  useEffect(() => {
    return () => {
      if (wsRef.current) {
        wsRef.current.close();
        wsRef.current = null;
      }
    };
  }, []);

  return { connect, disconnect, isConnected, send };
}
