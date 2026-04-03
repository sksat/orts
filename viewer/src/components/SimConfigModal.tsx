import type { SimConfigPayload } from "./SimConfigForm.js";
import { SimConfigForm } from "./SimConfigForm.js";
import styles from "./SimConfigModal.module.css";

interface SimConfigModalProps {
  isOpen: boolean;
  onStart: (config: SimConfigPayload) => void;
  onClose: () => void;
}

export function SimConfigModal({ isOpen, onStart, onClose }: SimConfigModalProps) {
  if (!isOpen) return null;

  return (
    <div className={styles.backdrop} onClick={onClose}>
      <div className={styles.modal} onClick={(e) => e.stopPropagation()}>
        <SimConfigForm onStart={onStart} />
      </div>
    </div>
  );
}
