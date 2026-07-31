/*
 * Vegcord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2023 Vendicated and Vencord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import { useState } from "@vencord/types/webpack/common";

import { SettingsComponent } from "./Settings";
import { VegcordSettingsSwitch } from "./VegcordSettingsSwitch";

export const AutoStartToggle: SettingsComponent = ({ settings }) => {
    const [autoStartEnabled, setAutoStartEnabled] = useState(VegcordNative.autostart.isEnabled());

    return (
        <>
            <VegcordSettingsSwitch
                title="Start With System"
                description="Automatically start Vegcord on computer start-up"
                value={autoStartEnabled}
                onChange={async v => {
                    await VegcordNative.autostart[v ? "enable" : "disable"]();
                    setAutoStartEnabled(v);
                }}
            />

            <VegcordSettingsSwitch
                title="Auto Start Minimized"
                description={"Start Vegcord minimized when starting with system"}
                value={settings.autoStartMinimized ?? false}
                onChange={v => (settings.autoStartMinimized = v)}
                disabled={!autoStartEnabled}
            />
        </>
    );
};
