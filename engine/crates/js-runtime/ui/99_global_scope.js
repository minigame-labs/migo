// Global scope registration for host_v8_ui APIs (api-system feature gate).

import * as interaction from 'ext:host_v8_ui/01_interaction.js';
import * as buttonsApi from 'ext:host_v8_ui/02_buttons.js';
import * as pageManagerApi from 'ext:host_v8_ui/03_page_manager.js';

import { primordials, core } from "ext:core/mod.js";
const { ObjectDefineProperties } = primordials;

ObjectDefineProperties(globalThis, {
    // UI Interaction
    showToast: core.propNonEnumerable(interaction.showToast),
    hideToast: core.propNonEnumerable(interaction.hideToast),
    showModal: core.propNonEnumerable(interaction.showModal),
    _internalOnModalResult: core.propNonEnumerable(interaction._internalOnModalResult),
    showLoading: core.propNonEnumerable(interaction.showLoading),
    hideLoading: core.propNonEnumerable(interaction.hideLoading),
    showActionSheet: core.propNonEnumerable(interaction.showActionSheet),
    _internalOnActionSheetResult: core.propNonEnumerable(interaction._internalOnActionSheetResult),

    // UI Buttons
    createUserInfoButton: core.propNonEnumerable(buttonsApi.createUserInfoButton),
    createGameClubButton: core.propNonEnumerable(buttonsApi.createGameClubButton),
    createFeedbackButton: core.propNonEnumerable(buttonsApi.createFeedbackButton),
    getMenuButtonBoundingClientRect: core.propNonEnumerable(buttonsApi.getMenuButtonBoundingClientRect),

    // Page Manager
    createPageManager: core.propNonEnumerable(pageManagerApi.createPageManager),
});
