import { useEffect, useState } from "react";
import { isTauri } from "@/lib/tauri-ipc";

export type Theme = "light" | "dark";

export function getSystemTheme(): Theme {
  if (typeof window === "undefined") return "dark";
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

export function applyTheme(theme: Theme): void {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  if (theme === "dark") {
    root.classList.add("dark");
    root.classList.remove("light");
    root.style.colorScheme = "dark";
  } else {
    root.classList.remove("dark");
    root.classList.add("light");
    root.style.colorScheme = "light";
  }
}

export function useTheme() {
  const [theme, setTheme] = useState<Theme>(() => {
    if (typeof document !== "undefined") {
      if (document.documentElement.classList.contains("dark")) return "dark";
      if (document.documentElement.classList.contains("light")) return "light";
    }
    return getSystemTheme();
  });

  useEffect(() => {
    // 1. Apply system theme on initial mount
    const initialTheme = getSystemTheme();
    setTheme(initialTheme);
    applyTheme(initialTheme);

    // 2. Listen to system preference changes via matchMedia
    const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
    const handleMediaChange = (e: MediaQueryListEvent) => {
      const nextTheme: Theme = e.matches ? "dark" : "light";
      setTheme(nextTheme);
      applyTheme(nextTheme);
    };

    if (mediaQuery.addEventListener) {
      mediaQuery.addEventListener("change", handleMediaChange);
    } else {
      mediaQuery.addListener(handleMediaChange);
    }

    // 3. In Tauri desktop app on macOS, listen for native theme changes
    let unlistenTauri: (() => void) | undefined;
    if (isTauri()) {
      import("@tauri-apps/api/window")
        .then(({ getCurrentWindow }) => {
          const appWindow = getCurrentWindow();
          void appWindow.theme().then((nativeTheme) => {
            if (nativeTheme === "dark" || nativeTheme === "light") {
              setTheme(nativeTheme);
              applyTheme(nativeTheme);
            }
          });

          return appWindow.onThemeChanged(({ payload: nativeTheme }) => {
            if (nativeTheme === "dark" || nativeTheme === "light") {
              setTheme(nativeTheme);
              applyTheme(nativeTheme);
            }
          });
        })
        .then((unlisten) => {
          unlistenTauri = unlisten;
        })
        .catch(() => {
          // Window theme API fallback handled by matchMedia
        });
    }

    return () => {
      if (mediaQuery.removeEventListener) {
        mediaQuery.removeEventListener("change", handleMediaChange);
      } else {
        mediaQuery.removeListener(handleMediaChange);
      }
      if (unlistenTauri) {
        unlistenTauri();
      }
    };
  }, []);

  return {
    theme,
    isDark: theme === "dark",
    isLight: theme === "light",
  };
}
