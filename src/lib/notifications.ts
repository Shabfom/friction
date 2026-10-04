import { invoke } from "@tauri-apps/api/core";
import { isTauri } from "./tauri-ipc";

/**
 * In the desktop app, notifications are sent natively by the Rust proxy the
 * moment it holds or blocks a request, so they arrive even when the window is
 * in the background. These helpers cover the permission prompt and the
 * "send test alert" button, plus the browser preview.
 */
export async function requestNotificationPermission(): Promise<string> {
  if (isTauri()) {
    try {
      return await invoke<string>("request_notification_permission");
    } catch (err) {
      console.warn("[Friction] notification permission check failed:", err);
      return "default";
    }
  }
  if (typeof Notification === "undefined") return "denied";
  if (Notification.permission === "default") {
    try {
      return await Notification.requestPermission();
    } catch {
      return "default";
    }
  }
  return Notification.permission;
}

export async function sendSystemNotification(title: string, body: string): Promise<boolean> {
  if (isTauri()) {
    try {
      return await invoke<boolean>("send_native_notification", { title, body });
    } catch (err) {
      console.warn("[Friction] native notification failed:", err);
      return false;
    }
  }
  if (typeof Notification !== "undefined" && Notification.permission === "granted") {
    new Notification(title, { body, tag: "friction-intercept" });
    return true;
  }
  return false;
}
