import { invoke } from '@tauri-apps/api/core';

type Size = { width: number; height: number };
export interface DisplayEnvironment {
  text_scale_percent: number | null;
  fast_startup_configured: boolean | null;
  runtime_version: string | null;
  monitors: {
    name: string | null;
    primary: boolean;
    size_physical: Size;
    position_physical: { x: number; y: number };
    scale_factor: number;
  }[];
  windows: {
    label: string;
    visible: boolean | null;
    minimized: boolean | null;
    monitor: string | null;
    scale_factor: number | null;
    client_physical: Size | null;
    webview: {
      zoom_factor: number;
      rasterization_scale: number;
      detects_system_scale: boolean;
      bounds_mode: number;
      bounds_physical: Size;
    } | null;
  }[];
  saved_appearance: unknown;
  errors: string[];
}

export interface RuntimeStatus {
  os_version: string;
  webview_version: string;
  elevated: boolean;
  input_mode: string;
  configured_input_mode: string;
  interception_installed: string;
  dd_hid_installed: string;
  hid_stick_present: boolean;
  scheduler_hp_degraded: boolean;
  resources_ok: boolean;
  missing_resources: string[];
}

export interface LiveEnvironment {
  captured_at: string;
  status: RuntimeStatus | null;
  display: DisplayEnvironment | null;
  frontend: {
    viewport_css: Size;
    device_pixel_ratio: number;
    visual_viewport_scale: number | null;
  };
  errors: string[];
}

export async function collectLiveEnvironment(): Promise<LiveEnvironment> {
  const [status, display] = await Promise.allSettled([
    invoke<RuntimeStatus>('get_app_status'),
    invoke<DisplayEnvironment>('get_display_environment'),
  ]);
  return {
    captured_at: new Date().toISOString(),
    status: status.status === 'fulfilled' ? status.value : null,
    display: display.status === 'fulfilled' ? display.value : null,
    frontend: {
      viewport_css: { width: window.innerWidth, height: window.innerHeight },
      device_pixel_ratio: window.devicePixelRatio,
      visual_viewport_scale: window.visualViewport?.scale ?? null,
    },
    errors: [status, display].flatMap((result, index) =>
      result.status === 'rejected'
        ? [`${index === 0 ? 'status' : 'display'}: ${String(result.reason)}`]
        : [],
    ),
  };
}
