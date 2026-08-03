const DIRECT_STATUS_COMMANDS = new Set([
  "open_log_folder",
  "install_update",
  "open_repository",
]);

export function normalizePayload(command, payload) {
  if (DIRECT_STATUS_COMMANDS.has(command)) {
    return { status: payload };
  }
  if (command === "list_schedule_rules") {
    return { schedule: payload };
  }
  if (command === "get_shutdown_state") {
    return { shutdown: payload };
  }
  if (command === "get_settings") {
    return { settings: payload };
  }
  if (command === "get_monitor_snapshot") {
    return { monitor: payload };
  }
  return payload;
}

export function mergePayload(previous, payload) {
  return {
    appVersion: payload.appVersion ?? previous.appVersion,
    status: payload.status ?? previous.status,
    control: payload.control ?? previous.control,
    schedule: payload.schedule ?? previous.schedule,
    shutdown: payload.shutdown ?? previous.shutdown,
    settings: payload.settings ?? previous.settings,
    update: payload.update ?? previous.update,
    monitor: payload.monitor ?? previous.monitor,
  };
}
