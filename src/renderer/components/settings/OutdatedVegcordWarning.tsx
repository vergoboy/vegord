/*
 * Vegcord, a desktop app aiming to give you a snappier Discord Experience
 * Copyright (c) 2025 Vendicated and Vegcord contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import { Button, Card, HeadingTertiary, Paragraph } from "@vencord/types/components";
import { useAwaiter } from "@vencord/types/utils";

import { cl } from "./Settings";

export function OutdatedVegcordWarning() {
    const [isOutdated] = useAwaiter(VegcordNative.app.isOutdated);

    if (!isOutdated) return null;

    return (
        <Card variant="warning" className={cl("updater-card")}>
            <HeadingTertiary>Your Vegcord is outdated!</HeadingTertiary>
            <Paragraph>Staying up to date is important for security and stability.</Paragraph>

            <Button onClick={() => VegcordNative.app.openUpdater()} variant="secondary">
                Open Updater
            </Button>
        </Card>
    );
}
