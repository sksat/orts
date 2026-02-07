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
}

type ServerMessage = StateMessage | InfoMessage;

export interface UseWebSocketOptions {
  /** WebSocket server URL, e.g. "ws://localhost:9001". */
  url: string;
  /** Called for each orbit state update received from the server. */
  onState: (state: OrbitPoint) => void;
  /** Called when the server sends simulation metadata (on connect). */
  onInfo: (info: SimInfo) => void;
}

export interface UseWebSocketReturn {
  /** Open a WebSocket connection to the configured URL. */
  connect: () => void;
  /** Close the active WebSocket connection. */
  disconnect: () => void;
  /** Whether a WebSocket connection is currently open. */
  isConnected: boolean;
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
          });
        }
      } catch {
        // Silently ignore malformed messages
      }
    });
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

  return { connect, disconnect, isConnected };
}
