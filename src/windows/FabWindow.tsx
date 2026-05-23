import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

export default function FabWindow() {
  const downPosRef = useRef<{ x: number; y: number } | null>(null);
  const draggedRef = useRef(false);

  useEffect(() => {
    const handleDown = (e: MouseEvent) => {
      if (e.button !== 0) return;
      downPosRef.current = { x: e.screenX, y: e.screenY };
      draggedRef.current = false;
    };

    const handleMove = async (e: MouseEvent) => {
      if (e.buttons !== 1) return;
      if (!downPosRef.current || draggedRef.current) return;
      const dx = Math.abs(e.screenX - downPosRef.current.x);
      const dy = Math.abs(e.screenY - downPosRef.current.y);
      if (dx > 4 || dy > 4) {
        draggedRef.current = true;
        try {
          await getCurrentWindow().startDragging();
        } catch (err) {
          console.error("startDragging failed:", err);
        }
      }
    };

    const handleUp = (e: MouseEvent) => {
      if (e.button !== 0) return;
      const wasClick = downPosRef.current !== null && !draggedRef.current;
      downPosRef.current = null;
      if (wasClick) {
        invoke("open_chat").catch((err) => console.error("open_chat failed:", err));
      }
    };

    const handleContextMenu = (e: MouseEvent) => e.preventDefault();

    document.addEventListener("mousedown", handleDown);
    document.addEventListener("mousemove", handleMove);
    document.addEventListener("mouseup", handleUp);
    document.addEventListener("contextmenu", handleContextMenu);

    return () => {
      document.removeEventListener("mousedown", handleDown);
      document.removeEventListener("mousemove", handleMove);
      document.removeEventListener("mouseup", handleUp);
      document.removeEventListener("contextmenu", handleContextMenu);
    };
  }, []);

  return (
    <div className="fab" title="Clique para abrir · Arraste para mover" aria-label="Artemis">
      <img src="/foguete.png" width="40" height="40" alt="Artemis" draggable={false} />
    </div>
  );
}
