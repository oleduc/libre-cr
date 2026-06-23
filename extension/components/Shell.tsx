import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";

import type { PanelGeometry } from "../utils/daemon/storage";
import { getPanelPosition, setPanelPosition } from "../utils/daemon/storage";

export interface ShellProps {
  prUrl: string;
  children: ReactNode;
  initialPosition?: { x: number; y: number; width: number; height: number };
}

/**
 * Keep at least this much of the panel inside the viewport so a drag can
 * never strand it off-screen (I13).
 */
const VIEWPORT_MARGIN = 40;

function clampToViewport(g: PanelGeometry): PanelGeometry {
  const maxX = Math.max(0, window.innerWidth - VIEWPORT_MARGIN);
  const maxY = Math.max(0, window.innerHeight - VIEWPORT_MARGIN);
  return {
    ...g,
    x: Math.min(Math.max(0, g.x), maxX),
    y: Math.min(Math.max(0, g.y), maxY),
  };
}

export function Shell({ prUrl, children, initialPosition }: ShellProps) {
  const [pos, setPos] = useState(initialPosition);
  const ref = useRef<HTMLDivElement | null>(null);
  const dragState = useRef<{ x: number; y: number; px: number; py: number } | null>(null);
  // Latest dragged geometry — read on mouseup so persisting doesn't have to
  // happen inside a state-updater (side-effect-free under StrictMode).
  const lastDragPos = useRef<PanelGeometry | null>(null);
  // Active window listeners, so unmount mid-drag can remove them (I13).
  const dragListeners = useRef<{
    move: (ev: MouseEvent) => void;
    up: () => void;
  } | null>(null);

  useEffect(() => {
    let cancelled = false;
    void getPanelPosition(prUrl).then((p) => {
      if (!cancelled && p) setPos(clampToViewport(p));
    });
    return () => {
      cancelled = true;
    };
  }, [prUrl]);

  // Remove any in-flight drag listeners if the tree unmounts mid-drag.
  useEffect(() => {
    return () => {
      const l = dragListeners.current;
      if (l) {
        window.removeEventListener("mousemove", l.move);
        window.removeEventListener("mouseup", l.up);
        dragListeners.current = null;
      }
      dragState.current = null;
    };
  }, []);

  const onTitleDown = (e: React.MouseEvent<HTMLDivElement>) => {
    const el = ref.current;
    if (!el || dragListeners.current) return;
    const rect = el.getBoundingClientRect();
    dragState.current = {
      x: e.clientX,
      y: e.clientY,
      px: rect.left,
      py: rect.top,
    };
    const move = (ev: MouseEvent) => {
      const st = dragState.current;
      if (!st) return;
      const nx = st.px + (ev.clientX - st.x);
      const ny = st.py + (ev.clientY - st.y);
      const next: PanelGeometry = {
        x: nx,
        y: ny,
        width: lastDragPos.current?.width ?? pos?.width ?? rect.width,
        height: lastDragPos.current?.height ?? pos?.height ?? rect.height,
      };
      lastDragPos.current = next;
      setPos(next);
    };
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      dragListeners.current = null;
      dragState.current = null;
      const dragged = lastDragPos.current;
      lastDragPos.current = null;
      if (dragged) {
        // Clamp so the titlebar always stays reachable inside the viewport.
        const clamped = clampToViewport(dragged);
        setPos(clamped);
        void setPanelPosition(prUrl, clamped);
      }
    };
    dragListeners.current = { move, up };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  const style = pos
    ? {
        top: pos.y,
        left: pos.x,
        right: "auto",
        width: pos.width || undefined,
        height: pos.height || undefined,
      }
    : undefined;

  return (
    <div
      ref={ref}
      className="libre-cr-shell"
      style={style}
      data-pr-url={prUrl}
      onMouseDownCapture={(e) => {
        const t = e.target as HTMLElement;
        if (t.closest(".libre-cr-titlebar") && !t.closest("button")) {
          onTitleDown(e);
        }
      }}
    >
      {children}
    </div>
  );
}
