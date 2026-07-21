// Global scope registration for host_v8_payment APIs (api-commerce feature gate).

import * as paymentApi from 'ext:host_v8_payment/01_payment.js';

import { primordials, core } from "ext:core/mod.js";
const { ObjectDefineProperties } = primordials;

ObjectDefineProperties(globalThis, {
    // Payment
    checkIsSupportMidasPayment: core.propNonEnumerable(paymentApi.checkIsSupportMidasPayment),
    requestMidasPayment: core.propNonEnumerable(paymentApi.requestMidasPayment),
    requestMidasPaymentGameItem: core.propNonEnumerable(paymentApi.requestMidasPaymentGameItem),
    _internalOnMidasPaymentResult: core.propNonEnumerable(paymentApi._internalOnMidasPaymentResult),
    _internalOnMidasPaymentGameItemResult: core.propNonEnumerable(paymentApi._internalOnMidasPaymentGameItemResult),
});
