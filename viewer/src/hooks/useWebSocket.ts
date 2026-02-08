import { useRef, useState, useCallback, useEffect } from "react";
import { OrbitPoint } from "../orbit.js";

/**
 * Simulation metadata sent by the server on initial connection.
 *
 * Corresponds to the `{"type":"info",...}` message from
 * `orts-cli --serve`.
 */
export interface SimInfo {
  mu: number;
  altitude: number;
  period: number;
  dt: number;
  output_interval: number;
  stream_interval: number;
  central_body: string;
  central_body_radius: number;
  /** Julian Date of the simulation epoch, or null if not set. */
  epoch_jd: number | null;
}

/**
 * Raw state message received over the WebSocket.
 * The server sends position as [x, y, z] and velocity as [vx, vy, vz].
 */
interface StateMessage {
  type: "state";
  t: number;
  position: [number, number, number];
  velocity: [number, number, number];
}

/** Raw info message received over the WebSocket. */
interface InfoMessage {
  type: "info";
  mu: number;
  altitude: number;
  period: number;
  dt: number;
  output_interval: number;
  stream_interval?: number;
  central_body?: string;
  central_body_radius?: number;
  epoch_jd?: number | null;
}

interface HistoryMessage {
  type: "history";
  states: Array<{ t: number; position: [number, number, number]; velocity: [number, number, number] }>;
}

interface HistoryDetailMessage {
  type: "history_detail";
  states: Array<{ t: number; position: [number, number, number]; velocity: [number, number, number] }>;
}

interface HistoryDetailCompleteMessage {
  type: "history_detail_complete";
}

interface QueryRangeResponseMessage {
  type: "query_range_response";
  t_min: number;
  t_max: number;
  states: Array<{ t: number; position: [number, number, number]; velocity: [number, number, number] }>;
}

type ServerMessage = StateMessage | InfoMessage | HistoryMessage | HistoryDetailMessage | HistoryDetailCompleteMessage | QueryRangeResponseMessage;

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
  const onStateRef = useRef(options.onState);
  const onInfoRef = useRef(options.onInfo);
  onStateRef.current = options.onState;
  onInfoRef.current = options.onInfo;
  const onHistoryRef = useRef(options.onHistory);
  const onHistoryDetailRef = useRef(options.onHistoryDetail);
  const onHistoryDetailCompleteRef = useRef(options.onHistoryDetailComplete);
  const onQueryRangeResponseRef = useRef(options.onQueryRangeResponse);
  onHistoryRef.current = options.onHistory;
  onHistoryDetailRef.current = options.onHistoryDetail;
  onHistoryDetailCompleteRef.current = options.onHistoryDetailComplete;
  onQueryRangeResponseRef.current = options.onQueryRangeResponse;

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
      setIsConnected(true);
    });

    ws.addEventListener("close", () => {
      setIsConnected(false);
      wsRef.current = null;
    });

    ws.addEventListener("error", () => {
      // The close event will fire after error, which resets state.
      // Nothing extra to do here.
    });

    ws.addEventListener("message", (event: MessageEvent) => {
      try {
        const msg = JSON.parse(event.data as string) as ServerMessage;

        if (msg.type === "state") {
          const stateMsg = msg as StateMessage;
          const point: OrbitPoint = {
            t: stateMsg.t,
            x: stateMsg.position[0],
            y: stateMsg.position[1],
            z: stateMsg.position[2],
            vx: stateMsg.velocity[0],
            vy: stateMsg.velocity[1],
            vz: stateMsg.velocity[2],
          };
          onStateRef.current(point);
        } else if (msg.type === "info") {
          const infoMsg = msg as InfoMessage;
          onInfoRef.current({
            mu: infoMsg.mu,
            altitude: infoMsg.altitude,
            period: infoMsg.period,
            dt: infoMsg.dt,
            output_interval: infoMsg.output_interval,
            stream_interval: infoMsg.stream_interval ?? infoMsg.output_interval,
            central_body: infoMsg.central_body ?? "earth",
            central_body_radius: infoMsg.central_body_radius ?? 6378.137,
            epoch_jd: infoMsg.epoch_jd ?? null,
          });
        } else if (msg.type === "history" || msg.type === "history_detail") {
          const histMsg = msg as HistoryMessage | HistoryDetailMessage;
          const points: OrbitPoint[] = histMsg.states.map((s) => ({
            t: s.t,
            x: s.position[0],
            y: s.position[1],
            z: s.position[2],
            vx: s.velocity[0],
            vy: s.velocity[1],
            vz: s.velocity[2],
          }));
          if (msg.type === "history") {
            onHistoryRef.current(points);
          } else {
            onHistoryDetailRef.current(points);
          }
        } else if (msg.type === "history_detail_complete") {
          onHistoryDetailCompleteRef.current();
        } else if (msg.type === "query_range_response") {
          const qrMsg = msg as QueryRangeResponseMessage;
          const points: OrbitPoint[] = qrMsg.states.map((s) => ({
            t: s.t,
            x: s.position[0],
            y: s.position[1],
            z: s.position[2],
            vx: s.velocity[0],
            vy: s.velocity[1],
            vz: s.velocity[2],
          }));
          onQueryRangeResponseRef.current?.({
            tMin: qrMsg.t_min,
            tMax: qrMsg.t_max,
            points,
          });
        }
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
