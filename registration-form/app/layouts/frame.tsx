import { createContext, useEffect, useState } from "react";

export type StepFrame = "ipad" | "web" | "phone";

export const StepFrameContext = createContext<StepFrame | undefined>(undefined);

function matches(query: string): boolean {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") return false;
  return window.matchMedia(query).matches;
}

export function kioskFrame(): "ipad" | "web" {
  if (typeof window === "undefined") return "web";
  const width = window.innerWidth;
  const short = Math.min(width, window.innerHeight);
  const touch = matches("(pointer: coarse)");
  if (touch && short >= 768) return "ipad";
  if (width >= 768 && (matches("(pointer: fine)") || !touch)) return "web";
  return "ipad";
}

export function useKioskFrame(): "ipad" | "web" {
  const [frame, setFrame] = useState(kioskFrame);
  useEffect(() => {
    const apply = () => setFrame(kioskFrame());
    window.addEventListener("resize", apply);
    return () => window.removeEventListener("resize", apply);
  }, []);
  return frame;
}
