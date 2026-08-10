/*
 * vegord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2026 Vendicated and vegord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import { contextBridge, ipcRenderer } from "electron/renderer";
import { IpcEvents } from "shared/IpcEvents";

contextBridge.exposeInMainWorld("vegordDebug", {
    getStatus: (): Promise<unknown> => ipcRenderer.invoke(IpcEvents.DEBUG_GET_STATUS),
    getRecentLogs: (): Promise<string[]> => ipcRenderer.invoke(IpcEvents.DEBUG_GET_LOGS),
    clearLog: (): Promise<boolean> => ipcRenderer.invoke(IpcEvents.DEBUG_CLEAR_LOG),
    rescanDoH: (): Promise<boolean> => ipcRenderer.invoke(IpcEvents.DEBUG_RESCAN),
    copyLog: (text: string): Promise<boolean> => ipcRenderer.invoke(IpcEvents.DEBUG_COPY, text),
    onLogLine(callback: (line: string) => void) {
        const listener = (_: unknown, line: string) => callback(line);
        ipcRenderer.on(IpcEvents.DEBUG_LOG_LINE, listener);
        return () => {
            ipcRenderer.removeListener(IpcEvents.DEBUG_LOG_LINE, listener);
        };
    },
    openExternal(url: string) {
        ipcRenderer.send("open-external", url);
    }
});
