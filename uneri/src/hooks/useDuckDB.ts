import { useState, useEffect, useRef } from "react";
import type { AsyncDuckDB, AsyncDuckDBConnection } from "@duckdb/duckdb-wasm";
import { initDuckDB } from "../db/duckdb.js";
import { createTable } from "../db/store.js";
import type { TableSchema } from "../types.js";

export interface UseDuckDBReturn {
  db: AsyncDuckDB | null;
  conn: AsyncDuckDBConnection | null;
  isReady: boolean;
  error: string | null;
}

export function useDuckDB(schema: TableSchema): UseDuckDBReturn {
  const [db, setDb] = useState<AsyncDuckDB | null>(null);
  const [conn, setConn] = useState<AsyncDuckDBConnection | null>(null);
  const [error, setError] = useState<string | null>(null);
  const initRef = useRef(false);

  useEffect(() => {
    if (initRef.current) return;
    initRef.current = true;

    (async () => {
      try {
        const database = await initDuckDB();
        const connection = await database.connect();
        await createTable(connection, schema);
        setDb(database);
        setConn(connection);
      } catch (e) {
        console.error("DuckDB init failed:", e);
        setError(e instanceof Error ? e.message : "DuckDB init failed");
      }
    })();
  }, []);

  return { db, conn, isReady: conn !== null, error };
}
