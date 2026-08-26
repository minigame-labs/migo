package com.migo.runtime;

import android.app.Activity;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Context;
import android.graphics.Color;
import android.graphics.PixelFormat;
import android.graphics.Typeface;
import android.graphics.drawable.GradientDrawable;
import android.os.Handler;
import android.os.Looper;
import android.text.SpannableStringBuilder;
import android.text.Spanned;
import android.text.style.ForegroundColorSpan;
import android.util.TypedValue;
import android.view.Gravity;
import android.view.MotionEvent;
import android.view.View;
import android.view.ViewGroup;
import android.view.WindowManager;
import android.widget.FrameLayout;
import android.widget.HorizontalScrollView;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;
import android.widget.Toast;

import com.migo.runtime.internal.NativeMethods;

import org.json.JSONArray;
import org.json.JSONObject;

import java.text.SimpleDateFormat;
import java.util.ArrayList;
import java.util.Date;
import java.util.List;
import java.util.Locale;

/**
 * On-device console log viewer, similar to vConsole.
 * <p>
 * Shows a floating button in the bottom-right corner. Tapping it opens a
 * full-width log panel with tab-based filtering (All / Log / Warn / Error).
 * <p>
 * Polls the native ring buffer periodically to display console.log output.
 *
 */
public class ConsoleLogView {

    private static final int BG_COLOR         = 0xF0111111;
    private static final int HEADER_BG        = 0xFF1A1A2E;
    private static final int TAB_ACTIVE_BG    = 0xFF16213E;
    private static final int TAB_TEXT_COLOR    = 0xFFCCCCCC;
    private static final int TAB_ACTIVE_COLOR = 0xFF4FC3F7;
    private static final int BTN_BG_COLOR     = 0xCC04AA6D;
    private static final int BTN_TEXT_COLOR    = 0xFFFFFFFF;
    private static final int SEPARATOR_COLOR  = 0xFF333333;

    private static final int LOG_COLOR   = 0xFFD4D4D4;
    private static final int WARN_COLOR  = 0xFFFFB74D;
    private static final int ERROR_COLOR = 0xFFEF5350;
    private static final int DEBUG_COLOR = 0xFF90CAF9;
    private static final int TIME_COLOR  = 0xFF757575;

    private static final int POLL_INTERVAL_MS = 500;
    private static final float PANEL_HEIGHT_RATIO = 0.45f;

    private final Context context;
    private final int sessionId;
    private final Handler handler = new Handler(Looper.getMainLooper());
    private final SimpleDateFormat timeFmt = new SimpleDateFormat("HH:mm:ss.SSS", Locale.US);

    // Floating button
    private TextView floatingBtn;
    private WindowManager wm;
    private WindowManager.LayoutParams btnParams;
    private boolean btnAttached = false;

    // Panel
    private FrameLayout panelRoot;
    private WindowManager.LayoutParams panelParams;
    private boolean panelAttached = false;
    private boolean panelVisible = false;

    private ScrollView logScrollView;
    private LinearLayout logContainer;
    private TextView[] tabButtons;

    // State
    private long cursor = 0;
    private int activeTab = 0; // 0=All, 1=Log(debug+info), 2=Warn, 3=Error
    private final List<LogEntry> allLogs = new ArrayList<>();
    private Runnable pollRunnable;
    private boolean destroyed = false;

    private static class LogEntry {
        final int level;
        final long timestamp;
        final String message;

        LogEntry(int level, long timestamp, String message) {
            this.level = level;
            this.timestamp = timestamp;
            this.message = message;
        }
    }

    public ConsoleLogView(Context context, int sessionId) {
        this.context = context;
        this.sessionId = sessionId;
        this.wm = (WindowManager) context.getSystemService(Context.WINDOW_SERVICE);
    }

    // ==================== Floating Button ====================

    public void attachButton(View anchor) {
        if (btnAttached || wm == null) return;

        floatingBtn = new TextView(context);
        floatingBtn.setText("vConsole");
        floatingBtn.setTextColor(BTN_TEXT_COLOR);
        floatingBtn.setTextSize(TypedValue.COMPLEX_UNIT_SP, 11f);
        floatingBtn.setTypeface(Typeface.DEFAULT_BOLD);
        floatingBtn.setPadding(dpToPx(10), dpToPx(5), dpToPx(10), dpToPx(5));
        floatingBtn.setGravity(Gravity.CENTER);

        GradientDrawable btnBg = new GradientDrawable();
        btnBg.setColor(BTN_BG_COLOR);
        btnBg.setCornerRadius(dpToPx(4));
        floatingBtn.setBackground(btnBg);

        floatingBtn.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                togglePanel(anchor);
            }
        });

        btnParams = new WindowManager.LayoutParams(
                WindowManager.LayoutParams.WRAP_CONTENT,
                WindowManager.LayoutParams.WRAP_CONTENT,
                WindowManager.LayoutParams.TYPE_APPLICATION_PANEL,
                WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE
                        | WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL,
                PixelFormat.TRANSLUCENT);
        btnParams.gravity = Gravity.BOTTOM | Gravity.END;
        btnParams.x = dpToPx(12);
        btnParams.y = dpToPx(12);
        btnParams.token = anchor.getWindowToken();

        wm.addView(floatingBtn, btnParams);
        btnAttached = true;

        startPolling();
    }

    public void detach() {
        destroyed = true;
        stopPolling();
        if (panelAttached) {
            try { wm.removeViewImmediate(panelRoot); } catch (Exception ignored) {}
            panelAttached = false;
        }
        if (btnAttached) {
            try { wm.removeViewImmediate(floatingBtn); } catch (Exception ignored) {}
            btnAttached = false;
        }
    }

    // ==================== Panel ====================

    private void togglePanel(View anchor) {
        if (panelVisible) {
            hidePanel();
        } else {
            showPanel(anchor);
        }
    }

    private void showPanel(View anchor) {
        if (panelAttached) {
            panelRoot.setVisibility(View.VISIBLE);
            panelVisible = true;
            return;
        }

        int screenHeight = context.getResources().getDisplayMetrics().heightPixels;
        int panelHeight = (int) (screenHeight * PANEL_HEIGHT_RATIO);

        panelRoot = new FrameLayout(context);

        LinearLayout mainLayout = new LinearLayout(context);
        mainLayout.setOrientation(LinearLayout.VERTICAL);

        GradientDrawable panelBg = new GradientDrawable();
        panelBg.setColor(BG_COLOR);
        mainLayout.setBackground(panelBg);

        // Header with tabs and close button
        LinearLayout header = createHeader();
        mainLayout.addView(header, new LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT));

        // Separator
        View sep = new View(context);
        sep.setBackgroundColor(SEPARATOR_COLOR);
        mainLayout.addView(sep, new LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, dpToPx(1)));

        // Log area
        logScrollView = new ScrollView(context);
        logScrollView.setFillViewport(true);

        logContainer = new LinearLayout(context);
        logContainer.setOrientation(LinearLayout.VERTICAL);
        logContainer.setPadding(dpToPx(6), dpToPx(4), dpToPx(6), dpToPx(4));
        logScrollView.addView(logContainer);

        mainLayout.addView(logScrollView, new LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f));

        // Bottom bar with Clear button
        LinearLayout bottomBar = createBottomBar();
        mainLayout.addView(bottomBar, new LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT));

        panelRoot.addView(mainLayout, new FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT));

        panelParams = new WindowManager.LayoutParams(
                WindowManager.LayoutParams.MATCH_PARENT,
                panelHeight,
                WindowManager.LayoutParams.TYPE_APPLICATION_PANEL,
                WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL
                        | WindowManager.LayoutParams.FLAG_WATCH_OUTSIDE_TOUCH,
                PixelFormat.TRANSLUCENT);
        panelParams.gravity = Gravity.BOTTOM;
        panelParams.token = anchor.getWindowToken();

        wm.addView(panelRoot, panelParams);
        panelAttached = true;
        panelVisible = true;

        refreshLogDisplay();
    }

    private void hidePanel() {
        if (panelAttached) {
            panelRoot.setVisibility(View.GONE);
            panelVisible = false;
        }
    }

    private LinearLayout createHeader() {
        LinearLayout header = new LinearLayout(context);
        header.setOrientation(LinearLayout.HORIZONTAL);
        header.setBackgroundColor(HEADER_BG);
        header.setGravity(Gravity.CENTER_VERTICAL);
        header.setPadding(dpToPx(4), dpToPx(2), dpToPx(4), dpToPx(2));

        String[] tabNames = {"All", "Log", "Warn", "Error"};
        tabButtons = new TextView[tabNames.length];

        for (int i = 0; i < tabNames.length; i++) {
            final int tabIdx = i;
            TextView tab = new TextView(context);
            tab.setText(tabNames[i]);
            tab.setTextSize(TypedValue.COMPLEX_UNIT_SP, 11f);
            tab.setTypeface(Typeface.DEFAULT_BOLD);
            tab.setPadding(dpToPx(10), dpToPx(6), dpToPx(10), dpToPx(6));

            tab.setOnClickListener(new View.OnClickListener() {
                @Override
                public void onClick(View v) {
                    setActiveTab(tabIdx);
                }
            });

            tabButtons[i] = tab;
            header.addView(tab);
        }

        // Spacer
        View spacer = new View(context);
        header.addView(spacer, new LinearLayout.LayoutParams(0,
                ViewGroup.LayoutParams.MATCH_PARENT, 1f));

        // Close button
        TextView closeBtn = new TextView(context);
        closeBtn.setText("X");
        closeBtn.setTextColor(TAB_TEXT_COLOR);
        closeBtn.setTextSize(TypedValue.COMPLEX_UNIT_SP, 13f);
        closeBtn.setTypeface(Typeface.DEFAULT_BOLD);
        closeBtn.setPadding(dpToPx(12), dpToPx(6), dpToPx(8), dpToPx(6));
        closeBtn.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                hidePanel();
            }
        });
        header.addView(closeBtn);

        updateTabStyles();
        return header;
    }

    private LinearLayout createBottomBar() {
        LinearLayout bar = new LinearLayout(context);
        bar.setOrientation(LinearLayout.HORIZONTAL);
        bar.setBackgroundColor(HEADER_BG);
        bar.setPadding(dpToPx(8), dpToPx(4), dpToPx(8), dpToPx(4));

        TextView clearBtn = new TextView(context);
        clearBtn.setText("Clear");
        clearBtn.setTextColor(TAB_ACTIVE_COLOR);
        clearBtn.setTextSize(TypedValue.COMPLEX_UNIT_SP, 11f);
        clearBtn.setPadding(dpToPx(8), dpToPx(4), dpToPx(8), dpToPx(4));
        clearBtn.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                allLogs.clear();
                refreshLogDisplay();
            }
        });
        bar.addView(clearBtn);

        TextView copyBtn = new TextView(context);
        copyBtn.setText("Copy");
        copyBtn.setTextColor(TAB_ACTIVE_COLOR);
        copyBtn.setTextSize(TypedValue.COMPLEX_UNIT_SP, 11f);
        copyBtn.setPadding(dpToPx(8), dpToPx(4), dpToPx(8), dpToPx(4));
        copyBtn.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                copyVisibleLogsToClipboard();
            }
        });
        bar.addView(copyBtn);

        // Spacer
        View spacer = new View(context);
        bar.addView(spacer, new LinearLayout.LayoutParams(0,
                ViewGroup.LayoutParams.MATCH_PARENT, 1f));

        // Log count
        TextView countLabel = new TextView(context);
        countLabel.setTextColor(TIME_COLOR);
        countLabel.setTextSize(TypedValue.COMPLEX_UNIT_SP, 10f);
        countLabel.setTag("log_count");
        bar.addView(countLabel);

        return bar;
    }

    private void setActiveTab(int tabIdx) {
        activeTab = tabIdx;
        updateTabStyles();
        refreshLogDisplay();
    }

    private void updateTabStyles() {
        if (tabButtons == null) return;
        for (int i = 0; i < tabButtons.length; i++) {
            if (i == activeTab) {
                GradientDrawable bg = new GradientDrawable();
                bg.setColor(TAB_ACTIVE_BG);
                bg.setCornerRadius(dpToPx(3));
                tabButtons[i].setBackground(bg);
                tabButtons[i].setTextColor(TAB_ACTIVE_COLOR);
            } else {
                tabButtons[i].setBackground(null);
                tabButtons[i].setTextColor(TAB_TEXT_COLOR);
            }
        }
    }

    // ==================== Polling ====================

    public void startPolling() {
        stopPolling();
        pollRunnable = new Runnable() {
            @Override
            public void run() {
                if (destroyed) return;
                pollLogs();
                handler.postDelayed(this, POLL_INTERVAL_MS);
            }
        };
        handler.post(pollRunnable);
    }

    public void stopPolling() {
        if (pollRunnable != null) {
            handler.removeCallbacks(pollRunnable);
            pollRunnable = null;
        }
    }

    private void pollLogs() {
        String json = null;
        try {
            json = NativeMethods.getConsoleLogs(sessionId, cursor);
        } catch (Exception ignored) {
        }
        if (json == null) return;

        try {
            JSONObject obj = new JSONObject(json);
            long newCursor = obj.optLong("cursor", cursor);
            JSONArray logs = obj.optJSONArray("logs");
            if (logs == null || logs.length() == 0) {
                cursor = newCursor;
                return;
            }

            boolean hasNew = false;
            for (int i = 0; i < logs.length(); i++) {
                JSONObject entry = logs.getJSONObject(i);
                int level = entry.optInt("l", 1);
                long ts = entry.optLong("t", 0);
                String msg = entry.optString("m", "");
                allLogs.add(new LogEntry(level, ts, msg));
                hasNew = true;
            }
            cursor = newCursor;

            if (hasNew && panelVisible) {
                refreshLogDisplay();
            }
        } catch (Exception ignored) {
        }
    }

    // ==================== Display ====================

    private void refreshLogDisplay() {
        if (logContainer == null) return;
        logContainer.removeAllViews();

        int count = 0;
        for (LogEntry entry : allLogs) {
            if (!matchesTab(entry.level)) continue;
            logContainer.addView(createLogRow(entry));
            count++;
        }

        // Update count label
        if (panelRoot != null) {
            View countView = panelRoot.findViewWithTag("log_count");
            if (countView instanceof TextView) {
                ((TextView) countView).setText(count + " / " + allLogs.size());
            }
        }

        // Auto-scroll to bottom
        logScrollView.post(new Runnable() {
            @Override
            public void run() {
                logScrollView.fullScroll(View.FOCUS_DOWN);
            }
        });
    }

    private boolean matchesTab(int level) {
        switch (activeTab) {
            case 0: return true;          // All
            case 1: return level <= 1;    // Log (debug=0, info=1)
            case 2: return level == 2;    // Warn
            case 3: return level == 3;    // Error
            default: return true;
        }
    }

    private View createLogRow(LogEntry entry) {
        TextView tv = new TextView(context);
        tv.setTypeface(Typeface.MONOSPACE);
        tv.setTextSize(TypedValue.COMPLEX_UNIT_SP, 10.5f);
        tv.setIncludeFontPadding(false);
        tv.setPadding(0, dpToPx(1), 0, dpToPx(1));

        SpannableStringBuilder ssb = new SpannableStringBuilder();

        // Timestamp
        String time = timeFmt.format(new Date(entry.timestamp));
        int start = ssb.length();
        ssb.append(time);
        ssb.setSpan(new ForegroundColorSpan(TIME_COLOR), start, ssb.length(),
                Spanned.SPAN_EXCLUSIVE_EXCLUSIVE);

        ssb.append(" ");

        // Level tag
        String tag;
        int tagColor;
        int msgColor;
        switch (entry.level) {
            case 0:  tag = "[D]"; tagColor = DEBUG_COLOR; msgColor = DEBUG_COLOR; break;
            case 2:  tag = "[W]"; tagColor = WARN_COLOR;  msgColor = WARN_COLOR;  break;
            case 3:  tag = "[E]"; tagColor = ERROR_COLOR; msgColor = ERROR_COLOR; break;
            default: tag = "[I]"; tagColor = LOG_COLOR;   msgColor = LOG_COLOR;   break;
        }

        start = ssb.length();
        ssb.append(tag);
        ssb.setSpan(new ForegroundColorSpan(tagColor), start, ssb.length(),
                Spanned.SPAN_EXCLUSIVE_EXCLUSIVE);

        ssb.append(" ");

        // Message (truncate at 2000 chars to avoid lag)
        start = ssb.length();
        String msg = entry.message.length() > 2000
                ? entry.message.substring(0, 2000) + "..."
                : entry.message;
        ssb.append(msg);
        ssb.setSpan(new ForegroundColorSpan(msgColor), start, ssb.length(),
                Spanned.SPAN_EXCLUSIVE_EXCLUSIVE);

        tv.setText(ssb);

        // Warn/Error rows get a subtle background
        if (entry.level == 2) {
            tv.setBackgroundColor(0x15FFFF00);
        } else if (entry.level == 3) {
            tv.setBackgroundColor(0x15FF0000);
        }

        return tv;
    }

    // ==================== Clipboard ====================

    private void copyVisibleLogsToClipboard() {
        StringBuilder sb = new StringBuilder();
        int count = 0;
        for (LogEntry entry : allLogs) {
            if (!matchesTab(entry.level)) continue;
            String time = timeFmt.format(new Date(entry.timestamp));
            String tag;
            switch (entry.level) {
                case 0:  tag = "[D]"; break;
                case 2:  tag = "[W]"; break;
                case 3:  tag = "[E]"; break;
                default: tag = "[I]"; break;
            }
            sb.append(time).append(' ').append(tag).append(' ').append(entry.message).append('\n');
            count++;
        }

        ClipboardManager cm = (ClipboardManager) context.getSystemService(Context.CLIPBOARD_SERVICE);
        if (cm == null) {
            Toast.makeText(context, "Clipboard unavailable", Toast.LENGTH_SHORT).show();
            return;
        }
        cm.setPrimaryClip(ClipData.newPlainText("vConsole", sb.toString()));
        Toast.makeText(context, "Copied " + count + " line(s)", Toast.LENGTH_SHORT).show();
    }

    // ==================== Util ====================

    private int dpToPx(int dp) {
        return (int) (dp * context.getResources().getDisplayMetrics().density + 0.5f);
    }
}
