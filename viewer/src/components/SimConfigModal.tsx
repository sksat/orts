import type { Pacing } from "../protocol/generated/Pacing.js";
import type { SimConfig } from "../protocol/generated/SimConfig.js";
import { SimConfigForm } from "./SimConfigForm.js";
import styles from "./SimConfigModal.module.css";

interface SimConfigModalProps {
  isOpen: boolean;
  onStart: (config: SimConfig, pacing?: Pacing) => void;
  /** The server's default pacing, or null while it is not known. */
  serverDefaultPacing: Pacing | null;
  onClose: () => void;
}

export function SimConfigModal({
  isOpen,
  onStart,
  serverDefaultPacing,
  onClose,
}: SimConfigModalProps) {
  if (!isOpen) return null;

  return (
    // biome-ignore lint/a11y/useKeyWithClickEvents: backdrop dismiss is mouse-only by design
    // biome-ignore lint/a11y/noStaticElementInteractions: backdrop overlay
    <div className={styles.backdrop} onClick={onClose}>
      {/* biome-ignore lint/a11y/noStaticElementInteractions: stopPropagation for modal content */}
      {/* biome-ignore lint/a11y/useKeyWithClickEvents: handled by backdrop */}
      <div className={styles.modal} onClick={(e) => e.stopPropagation()}>
        <SimConfigForm onStart={onStart} serverDefaultPacing={serverDefaultPacing} />
      </div>
    </div>
  );
}
