import { useRef, useState, useCallback, useEffect } from "react";
import { OrbitPoint } from "../orbit.js";

/** Per-satellite info from the server. */
export interface SatelliteInfo {
  id: string;
  name: string | null;
  altitude: number;
  period: number;
  /** Names of active perturbation force models (e.g. "drag", "srp"). */
  perturbations: string[];
}

/**
 * Simulation metadata sent by the server on initial connection.
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
 * Raw state message received over the WebSocket.
 * The server sends position as [x, y, z] and velocity as [vx, vy, vz].
 */
interface StateMessage {
  type: "state";
  satellite_id: string;
  t: number;
  position: [number, number, number];
  velocity: [number, number, number];
  semi_major_axis: number;
  eccentricity: number;
  inclination: number;
  raan: number;
  argument_of_periapsis: number;
  true_anomaly: number;
  accelerations?: Record<string, number>;
}

/** Raw info message received over the WebSocket. */
interface InfoMessage {
  type: "info";
  mu: number;
  dt: number;
  output_interval: number;
  stream_interval?: number;
  central_body?: string;
  central_body_radius?: number;
  epoch_jd?: number | null;
  satellites?: SatelliteInfoMsg[];
}

interface SatelliteInfoMsg {
  id: string;
  name?: string | null;
  altitude: number;
  period: number;
  perturbations?: string[];
}

interface HistoryStateMsg {
  satellite_id?: string;
  t: number;
  position: [number, number, number];
  velocity: [number, number, number];
  semi_major_axis: number;
  eccentricity: number;
  inclination: number;
  raan: number;
  argument_of_periapsis: number;
  true_anomaly: number;
  accelerations?: Record<string, number>;
}

interface HistoryMessage {
  type: "history";
  states: HistoryStateMsg[];
}

interface HistoryDetailMessage {
  type: "history_detail";
  states: HistoryStateMsg[];
}

interface HistoryDetailCompleteMessage {
  type: "history_detail_complete";
}

interface QueryRangeResponseMessage {
  type: "query_range_response";
  t_min: number;
  t_max: number;
  states: HistoryStateMsg[];
}

interface SimulationTerminatedMessage {
  type: "simulation_terminated";
  satellite_id: string;
  t: number;
  reason: string;
}

export type ServerMessage = StateMessage | InfoMessage | HistoryMessage | HistoryDetailMessage | HistoryDetailCompleteMessage | QueryRangeResponseMessage | SimulationTerminatedMessage;

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
  onHistoryDetail: (points: OrbitPoint[]) => void;
  onHistoryDetailComplete: () => void;
  /** Called when the server responds to a query_range request. */
  onQueryRangeResponse?: (response: QueryRangeResponse) => void;
  /** Called when a satellite's simulation terminates (collision, atmospheric entry, etc.). */
  onSimulationTerminated?: (satelliteId: string, t: number, reason: string) => void;
}

/** Callbacks for message dispatch (subset of UseWebSocketOptions used by dispatchServerMessage). */
export interface DispatchCallbacks {
  onState: (state: OrbitPoint) => void;
  onInfo?: (info: SimInfo) => void;
  onHistory?: (points: OrbitPoint[]) => void;
  onHistoryDetail?: (points: OrbitPoint[]) => void;
  onHistoryDetailComplete?: () => void;
  onQueryRangeResponse?: (response: QueryRangeResponse) => void;
  onSimulationTerminated?: (satelliteId: string, t: number, reason: string) => void;
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

function parseHistoryPoints(states: HistoryStateMsg[]): OrbitPoint[] {
  return states.map((s) => ({
    satelliteId: s.satellite_id,
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
    ...parseAccelerations(s.accelerations),
  }));
}

/**
 * Dispatch a parsed server message to the appropriate callback.
 * Extracted as a pure function for testability.
 */
export function dispatchServerMessage(
  msg: ServerMessage,
  callbacks: DispatchCallbacks,
): void {
  if (msg.type === "state") {
    const stateMsg = msg as StateMessage;
    callbacks.onState({
      satelliteId: stateMsg.satellite_id,
      t: stateMsg.t,
      x: stateMsg.position[0],
      y: stateMsg.position[1],
      z: stateMsg.position[2],
      vx: stateMsg.velocity[0],
      vy: stateMsg.velocity[1],
      vz: stateMsg.velocity[2],
      a: stateMsg.semi_major_axis,
      e: stateMsg.eccentricity,
      inc: stateMsg.inclination,
      raan: stateMsg.raan,
      omega: stateMsg.argument_of_periapsis,
      nu: stateMsg.true_anomaly,
      ...parseAccelerations(stateMsg.accelerations),
    });
  } else if (msg.type === "info") {
    const infoMsg = msg as InfoMessage;
    const satellites: SatelliteInfo[] = (infoMsg.satellites ?? []).map((s) => ({
      id: s.id,
      name: s.name ?? null,
      altitude: s.altitude,
      period: s.period,
      perturbations: s.perturbations ?? [],
    }));
    callbacks.onInfo?.({
      mu: infoMsg.mu,
      dt: infoMsg.dt,
      output_interval: infoMsg.output_interval,
      stream_interval: infoMsg.stream_interval ?? infoMsg.output_interval,
      central_body: infoMsg.central_body ?? "earth",
      central_body_radius: infoMsg.central_body_radius ?? 6378.137,
      epoch_jd: infoMsg.epoch_jd ?? null,
      satellites,
    });
  } else if (msg.type === "history" || msg.type === "history_detail") {
    const histMsg = msg as HistoryMessage | HistoryDetailMessage;
    const points = parseHistoryPoints(histMsg.states);
    if (msg.type === "history") {
      callbacks.onHistory?.(points);
    } else {
      callbacks.onHistoryDetail?.(points);
    }
  } else if (msg.type === "history_detail_complete") {
    callbacks.onHistoryDetailComplete?.();
  } else if (msg.type === "query_range_response") {
    const qrMsg = msg as QueryRangeResponseMessage;
    const points = parseHistoryPoints(qrMsg.states);
    callbacks.onQueryRangeResponse?.({
      tMin: qrMsg.t_min,
      tMax: qrMsg.t_max,
      points,
    });
  } else if (msg.type === "simulation_terminated") {
    const termMsg = msg as SimulationTerminatedMessage;
    callbacks.onSimulationTerminated?.(termMsg.satellite_id, termMsg.t, termMsg.reason);
  }
}

export interface UseWebSocketReturn {
  /** Open a WebSocket connection to the configured URL. */
  connect: () => void;
  /** Close the active WebSocket connection. */
  disconnect: () => void;
  /** Whether a WebSocket connection is currently open. */
  isConnected: boolean;
  /** Send a JSON message to the server. */
  send: (msg: Record<string, unknown>) => void;
}

/**
 * React hook for connecting to the Orts simulation WebSocket server.
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
    onHistoryDetail: options.onHistoryDetail,
    onHistoryDetailComplete: options.onHistoryDetailComplete,
    onQueryRangeResponse: options.onQueryRangeResponse,
    onSimulationTerminated: options.onSimulationTerminated,
  });
  callbacksRef.current = {
    onState: options.onState,
    onInfo: options.onInfo,
    onHistory: options.onHistory,
    onHistoryDetail: options.onHistoryDetail,
    onHistoryDetailComplete: options.onHistoryDetailComplete,
    onQueryRangeResponse: options.onQueryRangeResponse,
    onSimulationTerminated: options.onSimulationTerminated,
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

  const send = useCallback((msg: Record<string, unknown>) => {
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
