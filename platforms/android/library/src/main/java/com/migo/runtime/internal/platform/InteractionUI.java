package com.migo.runtime.internal.platform;

import android.app.Activity;
import android.app.AlertDialog;
import android.content.DialogInterface;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.ColorFilter;
import android.graphics.Paint;
import android.graphics.PixelFormat;
import android.graphics.RectF;
import android.graphics.drawable.Drawable;
import android.graphics.drawable.GradientDrawable;
import android.os.Handler;
import android.os.Looper;
import android.util.TypedValue;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.view.WindowManager;
import android.widget.FrameLayout;
import android.widget.LinearLayout;
import android.widget.ProgressBar;
import android.widget.TextView;

import com.migo.runtime.internal.CallbackCorrelation;
import com.migo.runtime.internal.NativeMethods;

import org.json.JSONObject;

/**
 * Native UI overlays for interaction APIs.
 * <p>
 * Implements showToast, hideToast, showModal, showLoading, hideLoading
 * using programmatic Android views (no XML layouts).
 *
 * @hide
 */
public final class InteractionUI {

    private static final Handler sHandler = new Handler(Looper.getMainLooper());

    // Current overlay references (for hide operations)
    private static View sToastOverlay;
    private static View sLoadingOverlay;
    private static Runnable sToastDismissRunnable;

    private InteractionUI() {}

    // ==================== Toast ====================

    public static void showToast(final Activity activity, final String json) {
        sHandler.post(() -> {
            try {
                JSONObject params = new JSONObject(json);
                String title = params.optString("title", "");
                String icon = params.optString("icon", "success");
                int duration = params.optInt("duration", 1500);
                boolean mask = params.optBoolean("mask", false);

                hideToastInternal(activity);

                View overlay = createOverlay(activity, title, icon, mask);
                addOverlay(activity, overlay);
                sToastOverlay = overlay;

                sToastDismissRunnable = () -> hideToastInternal(activity);
                sHandler.postDelayed(sToastDismissRunnable, duration);
            } catch (Exception e) {
                // Silently fail
            }
        });
    }

    public static void hideToast(final Activity activity) {
        sHandler.post(() -> hideToastInternal(activity));
    }

    private static void hideToastInternal(Activity activity) {
        if (sToastDismissRunnable != null) {
            sHandler.removeCallbacks(sToastDismissRunnable);
            sToastDismissRunnable = null;
        }
        if (sToastOverlay != null) {
            removeOverlay(activity, sToastOverlay);
            sToastOverlay = null;
        }
    }

    // ==================== Loading ====================

    public static void showLoading(final Activity activity, final String json) {
        sHandler.post(() -> {
            try {
                JSONObject params = new JSONObject(json);
                String title = params.optString("title", "");
                boolean mask = params.optBoolean("mask", false);

                hideLoadingInternal(activity);

                View overlay = createOverlay(activity, title, "loading", mask);
                addOverlay(activity, overlay);
                sLoadingOverlay = overlay;
            } catch (Exception e) {
                // Silently fail
            }
        });
    }

    public static void hideLoading(final Activity activity) {
        sHandler.post(() -> hideLoadingInternal(activity));
    }

    private static void hideLoadingInternal(Activity activity) {
        if (sLoadingOverlay != null) {
            removeOverlay(activity, sLoadingOverlay);
            sLoadingOverlay = null;
        }
    }

    // ==================== Modal ====================

    public static void showModal(final Activity activity, final int sessionId, final String json) {
        // Bound before the parse, not inside it: a malformed options string
        // still has to answer the request that sent it, and `requestIdOf`
        // reads ABSENT rather than throwing.
        final int requestId = CallbackCorrelation.requestIdOf(json);
        sHandler.post(() -> {
            try {
                JSONObject params = new JSONObject(json);
                String title = params.optString("title", "");
                String content = params.optString("content", "");
                boolean showCancel = params.optBoolean("showCancel", true);
                String cancelText = params.optString("cancelText", "\u53d6\u6d88"); // 取消
                String confirmText = params.optString("confirmText", "\u786e\u5b9a"); // 确定
                String cancelColor = params.optString("cancelColor", "#000000");
                String confirmColor = params.optString("confirmColor", "#576B95");

                showModalDialog(activity, sessionId, requestId, title, content, showCancel,
                        cancelText, confirmText, cancelColor, confirmColor);
            } catch (Exception e) {
                // Report cancel on error
                NativeMethods.onModalResult(sessionId, requestId, 0, 1);
            }
        });
    }

    private static void showModalDialog(Activity activity, int sessionId, int requestId,
                                         String title, String content, boolean showCancel,
                                         String cancelText, String confirmText,
                                         String cancelColor, String confirmColor) {
        if (activity == null || activity.isFinishing() || activity.isDestroyed()) {
            NativeMethods.onModalResult(sessionId, requestId, 0, 1);
            return;
        }

        // Build custom view for the dialog content
        float density = activity.getResources().getDisplayMetrics().density;
        int padding = (int) (24 * density);
        int topPadding = (int) (20 * density);

        LinearLayout layout = new LinearLayout(activity);
        layout.setOrientation(LinearLayout.VERTICAL);
        layout.setPadding(padding, topPadding, padding, (int) (8 * density));

        // Title
        if (title != null && !title.isEmpty()) {
            TextView titleView = new TextView(activity);
            titleView.setText(title);
            titleView.setTextSize(TypedValue.COMPLEX_UNIT_SP, 17);
            titleView.setTextColor(Color.BLACK);
            titleView.setGravity(Gravity.CENTER);
            titleView.getPaint().setFakeBoldText(true);
            LinearLayout.LayoutParams titleParams = new LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.MATCH_PARENT,
                    LinearLayout.LayoutParams.WRAP_CONTENT
            );
            titleParams.bottomMargin = (int) (8 * density);
            layout.addView(titleView, titleParams);
        }

        // Content
        if (content != null && !content.isEmpty()) {
            TextView contentView = new TextView(activity);
            contentView.setText(content);
            contentView.setTextSize(TypedValue.COMPLEX_UNIT_SP, 15);
            contentView.setTextColor(Color.parseColor("#888888"));
            contentView.setGravity(Gravity.CENTER);
            LinearLayout.LayoutParams contentParams = new LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.MATCH_PARENT,
                    LinearLayout.LayoutParams.WRAP_CONTENT
            );
            contentParams.topMargin = (int) (4 * density);
            contentParams.bottomMargin = (int) (12 * density);
            layout.addView(contentView, contentParams);
        }

        AlertDialog.Builder builder = new AlertDialog.Builder(activity);
        builder.setView(layout);
        builder.setCancelable(false);

        // Confirm button
        builder.setPositiveButton(confirmText, (dialog, which) ->
                NativeMethods.onModalResult(sessionId, requestId, 1, 0)
        );

        // Cancel button
        if (showCancel) {
            builder.setNegativeButton(cancelText, (dialog, which) ->
                    NativeMethods.onModalResult(sessionId, requestId, 0, 1)
            );
        }

        AlertDialog dialog = builder.create();
        dialog.show();

        // Apply button colors after show()
        try {
            dialog.getButton(DialogInterface.BUTTON_POSITIVE)
                    .setTextColor(Color.parseColor(confirmColor));
            if (showCancel) {
                dialog.getButton(DialogInterface.BUTTON_NEGATIVE)
                        .setTextColor(Color.parseColor(cancelColor));
            }
        } catch (Exception ignored) {
        }
    }

    // ==================== Action Sheet ====================

    public static void showActionSheet(final Activity activity, final int sessionId, final String json) {
        final int requestId = CallbackCorrelation.requestIdOf(json);
        sHandler.post(() -> {
            try {
                JSONObject params = new JSONObject(json);
                String alertText = params.optString("alertText", "");
                String itemColor = params.optString("itemColor", "#000000");
                org.json.JSONArray itemArray = params.getJSONArray("itemList");

                String[] items = new String[itemArray.length()];
                for (int i = 0; i < itemArray.length(); i++) {
                    items[i] = itemArray.getString(i);
                }

                showActionSheetDialog(activity, sessionId, requestId, alertText, items, itemColor);
            } catch (Exception e) {
                NativeMethods.onActionSheetResult(sessionId, requestId, -1);
            }
        });
    }

    private static void showActionSheetDialog(Activity activity, int sessionId, int requestId,
                                               String alertText, String[] items, String itemColor) {
        if (activity == null || activity.isFinishing() || activity.isDestroyed()) {
            NativeMethods.onActionSheetResult(sessionId, requestId, -1);
            return;
        }

        float density = activity.getResources().getDisplayMetrics().density;

        int itemColorParsed;
        try {
            itemColorParsed = Color.parseColor(itemColor);
        } catch (Exception e) {
            itemColorParsed = Color.BLACK;
        }

        // Root: full-screen semi-transparent background
        final FrameLayout root = new FrameLayout(activity);
        root.setLayoutParams(new FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT,
                FrameLayout.LayoutParams.MATCH_PARENT));
        root.setBackgroundColor(Color.parseColor("#80000000"));

        // Bottom container
        LinearLayout bottomContainer = new LinearLayout(activity);
        bottomContainer.setOrientation(LinearLayout.VERTICAL);
        int hMargin = (int) (8 * density);
        int bottomMargin = (int) (8 * density);

        FrameLayout.LayoutParams containerParams = new FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT,
                FrameLayout.LayoutParams.WRAP_CONTENT);
        containerParams.gravity = Gravity.BOTTOM;
        containerParams.leftMargin = hMargin;
        containerParams.rightMargin = hMargin;
        containerParams.bottomMargin = bottomMargin;

        // Items group (white rounded background)
        LinearLayout itemsGroup = new LinearLayout(activity);
        itemsGroup.setOrientation(LinearLayout.VERTICAL);
        GradientDrawable itemsBg = new GradientDrawable();
        itemsBg.setColor(Color.WHITE);
        itemsBg.setCornerRadius(12 * density);
        itemsGroup.setBackground(itemsBg);

        // Alert text header
        if (alertText != null && !alertText.isEmpty()) {
            TextView headerView = new TextView(activity);
            headerView.setText(alertText);
            headerView.setTextSize(TypedValue.COMPLEX_UNIT_SP, 13);
            headerView.setTextColor(Color.parseColor("#888888"));
            headerView.setGravity(Gravity.CENTER);
            headerView.setPadding(0, (int) (14 * density), 0, (int) (14 * density));
            itemsGroup.addView(headerView, new LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.MATCH_PARENT,
                    LinearLayout.LayoutParams.WRAP_CONTENT));

            // Divider after header
            View divider = new View(activity);
            divider.setBackgroundColor(Color.parseColor("#E5E5E5"));
            itemsGroup.addView(divider, new LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.MATCH_PARENT, 1));
        }

        // Item rows
        final int finalItemColor = itemColorParsed;
        for (int i = 0; i < items.length; i++) {
            final int index = i;

            TextView itemView = new TextView(activity);
            itemView.setText(items[i]);
            itemView.setTextSize(TypedValue.COMPLEX_UNIT_SP, 17);
            itemView.setTextColor(finalItemColor);
            itemView.setGravity(Gravity.CENTER);
            itemView.setPadding(0, (int) (14 * density), 0, (int) (14 * density));
            itemView.setClickable(true);

            TypedValue outValue = new TypedValue();
            activity.getTheme().resolveAttribute(android.R.attr.selectableItemBackground, outValue, true);
            itemView.setBackgroundResource(outValue.resourceId);

            itemView.setOnClickListener(v -> {
                removeOverlay(activity, root);
                NativeMethods.onActionSheetResult(sessionId, requestId, index);
            });

            itemsGroup.addView(itemView, new LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.MATCH_PARENT,
                    LinearLayout.LayoutParams.WRAP_CONTENT));

            // Divider between items (not after last)
            if (i < items.length - 1) {
                View divider = new View(activity);
                divider.setBackgroundColor(Color.parseColor("#E5E5E5"));
                itemsGroup.addView(divider, new LinearLayout.LayoutParams(
                        LinearLayout.LayoutParams.MATCH_PARENT, 1));
            }
        }

        bottomContainer.addView(itemsGroup, new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                LinearLayout.LayoutParams.WRAP_CONTENT));

        // Gap between items group and cancel button
        View gap = new View(activity);
        bottomContainer.addView(gap, new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, (int) (8 * density)));

        // Cancel button (separate rounded white background)
        TextView cancelBtn = new TextView(activity);
        cancelBtn.setText("\u53d6\u6d88"); // 取消
        cancelBtn.setTextSize(TypedValue.COMPLEX_UNIT_SP, 17);
        cancelBtn.setTextColor(Color.BLACK);
        cancelBtn.setGravity(Gravity.CENTER);
        cancelBtn.setPadding(0, (int) (14 * density), 0, (int) (14 * density));
        cancelBtn.setClickable(true);

        GradientDrawable cancelBg = new GradientDrawable();
        cancelBg.setColor(Color.WHITE);
        cancelBg.setCornerRadius(12 * density);
        cancelBtn.setBackground(cancelBg);

        cancelBtn.setOnClickListener(v -> {
            removeOverlay(activity, root);
            NativeMethods.onActionSheetResult(sessionId, requestId, -1);
        });

        bottomContainer.addView(cancelBtn, new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                LinearLayout.LayoutParams.WRAP_CONTENT));

        root.addView(bottomContainer, containerParams);

        // Tap on background to cancel
        root.setOnClickListener(v -> {
            removeOverlay(activity, root);
            NativeMethods.onActionSheetResult(sessionId, requestId, -1);
        });

        // Prevent click-through on bottom container
        bottomContainer.setOnClickListener(v -> { /* consume */ });

        addOverlay(activity, root);
    }

    // ==================== Overlay Helpers ====================

    private static View createOverlay(Activity activity, String title, String icon, boolean mask) {
        float density = activity.getResources().getDisplayMetrics().density;
        int boxSize = (int) (136 * density);
        int cornerRadius = (int) (12 * density);
        int iconSize = (int) (40 * density);
        int padding = (int) (20 * density);

        // Root: full-screen frame for centering (and optional mask)
        FrameLayout root = new FrameLayout(activity);
        FrameLayout.LayoutParams rootParams = new FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT,
                FrameLayout.LayoutParams.MATCH_PARENT
        );
        root.setLayoutParams(rootParams);

        if (mask) {
            root.setBackgroundColor(Color.TRANSPARENT);
            root.setClickable(true);
        } else {
            root.setClickable(false);
            root.setFocusable(false);
        }

        // Box: dark semi-transparent rounded container
        LinearLayout box = new LinearLayout(activity);
        box.setOrientation(LinearLayout.VERTICAL);
        box.setGravity(Gravity.CENTER);
        box.setPadding(padding, padding, padding, padding);

        GradientDrawable bg = new GradientDrawable();
        bg.setColor(Color.parseColor("#B2404040"));
        bg.setCornerRadius(cornerRadius);
        box.setBackground(bg);

        FrameLayout.LayoutParams boxParams = new FrameLayout.LayoutParams(boxSize, boxSize);
        boxParams.gravity = Gravity.CENTER;

        // Icon or ProgressBar
        boolean showIcon = !"none".equals(icon);
        if (showIcon) {
            if ("loading".equals(icon)) {
                ProgressBar progress = new ProgressBar(activity, null, android.R.attr.progressBarStyleLarge);
                progress.setIndeterminate(true);
                // Tint to white
                progress.getIndeterminateDrawable().setColorFilter(
                        Color.WHITE, android.graphics.PorterDuff.Mode.SRC_IN);
                LinearLayout.LayoutParams progressParams = new LinearLayout.LayoutParams(iconSize, iconSize);
                progressParams.gravity = Gravity.CENTER;
                progressParams.bottomMargin = (int) (12 * density);
                box.addView(progress, progressParams);
            } else {
                // Custom drawn icon (success checkmark or error X)
                View iconView = new View(activity) {
                    private final Paint iconPaint;
                    {
                        iconPaint = new Paint(Paint.ANTI_ALIAS_FLAG);
                        iconPaint.setColor(Color.WHITE);
                        iconPaint.setStyle(Paint.Style.STROKE);
                        iconPaint.setStrokeWidth(3 * density);
                        iconPaint.setStrokeCap(Paint.Cap.ROUND);
                    }

                    @Override
                    protected void onDraw(Canvas canvas) {
                        super.onDraw(canvas);

                        float w = getWidth();
                        float h = getHeight();
                        float cx = w / 2f;
                        float cy = h / 2f;
                        float r = Math.min(w, h) / 2f - 2 * density;

                        if ("success".equals(icon)) {
                            // Draw circle
                            canvas.drawCircle(cx, cy, r, iconPaint);
                            // Draw checkmark
                            float startX = cx - r * 0.35f;
                            float startY = cy + r * 0.05f;
                            float midX = cx - r * 0.05f;
                            float midY = cy + r * 0.35f;
                            float endX = cx + r * 0.4f;
                            float endY = cy - r * 0.25f;
                            canvas.drawLine(startX, startY, midX, midY, iconPaint);
                            canvas.drawLine(midX, midY, endX, endY, iconPaint);
                        } else if ("error".equals(icon)) {
                            // Draw circle
                            canvas.drawCircle(cx, cy, r, iconPaint);
                            // Draw X
                            float offset = r * 0.35f;
                            canvas.drawLine(cx - offset, cy - offset, cx + offset, cy + offset, iconPaint);
                            canvas.drawLine(cx + offset, cy - offset, cx - offset, cy + offset, iconPaint);
                        }
                    }
                };
                LinearLayout.LayoutParams iconParams = new LinearLayout.LayoutParams(iconSize, iconSize);
                iconParams.gravity = Gravity.CENTER;
                iconParams.bottomMargin = (int) (12 * density);
                box.addView(iconView, iconParams);
            }
        }

        // Title text
        if (title != null && !title.isEmpty()) {
            TextView titleView = new TextView(activity);
            titleView.setText(title);
            titleView.setTextColor(Color.WHITE);
            titleView.setTextSize(TypedValue.COMPLEX_UNIT_SP, 14);
            titleView.setGravity(Gravity.CENTER);
            titleView.setMaxLines(3);

            LinearLayout.LayoutParams textParams = new LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.WRAP_CONTENT,
                    LinearLayout.LayoutParams.WRAP_CONTENT
            );
            textParams.gravity = Gravity.CENTER;
            box.addView(titleView, textParams);
        }

        // If icon is "none", make box smaller (no icon, just text)
        if (!showIcon) {
            boxParams = new FrameLayout.LayoutParams(
                    FrameLayout.LayoutParams.WRAP_CONTENT,
                    FrameLayout.LayoutParams.WRAP_CONTENT
            );
            boxParams.gravity = Gravity.CENTER;
            box.setMinimumWidth((int) (120 * density));
            box.setMinimumHeight((int) (48 * density));
        }

        root.addView(box, boxParams);
        return root;
    }

    private static void addOverlay(Activity activity, View overlay) {
        if (activity == null || activity.isFinishing() || activity.isDestroyed()) return;
        try {
            ViewGroup decorView = (ViewGroup) activity.getWindow().getDecorView();
            decorView.addView(overlay);
        } catch (Exception ignored) {
        }
    }

    private static void removeOverlay(Activity activity, View overlay) {
        if (overlay == null) return;
        try {
            ViewGroup parent = (ViewGroup) overlay.getParent();
            if (parent != null) {
                parent.removeView(overlay);
            }
        } catch (Exception ignored) {
        }
    }
}
