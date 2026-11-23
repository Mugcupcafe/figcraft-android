package ai.storyteller.figcraft;

import android.Manifest;
import android.content.ClipData;
import android.content.ClipDescription;
import android.content.ClipboardManager;
import android.content.ContentValues;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.content.res.Configuration;
import android.database.Cursor;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Environment;
import android.os.SystemClock;
import android.provider.DocumentsContract;
import android.provider.MediaStore;
import android.provider.OpenableColumns;
import android.view.DragAndDropPermissions;
import android.view.DragEvent;
import android.view.MotionEvent;
import android.view.View;
import android.text.Editable;
import android.text.InputType;
import android.view.KeyEvent;
import android.view.ViewGroup;
import android.view.inputmethod.BaseInputConnection;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputMethodManager;

import androidx.core.app.ActivityCompat;
import androidx.core.content.ContextCompat;
import androidx.core.graphics.Insets;
import androidx.core.view.ViewCompat;
import androidx.core.view.WindowCompat;
import androidx.core.view.WindowInsetsCompat;
import androidx.core.view.WindowInsetsControllerCompat;

import com.google.androidgamesdk.GameActivity;

import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.lang.ref.WeakReference;
import java.net.Inet4Address;
import java.net.InetAddress;
import java.net.NetworkInterface;
import java.util.Enumeration;
import java.util.Locale;
import java.util.concurrent.ConcurrentLinkedQueue;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * Main Android tablet Activity for FigCraft (V6_hotfix).
 *
 * V6_hotfix additions:
 * - Toggleable Immersive Fullscreen mode (`set_immersive`) hiding/showing Android system bars.
 * - Clipboard MIME fallback via ClipDescription + 2 MiB text safety cap.
 */
public class FigCraftActivity extends GameActivity {
    private static final String SEP = "\u001F";
    private static final int REQ_OPEN_FILE = 1001;
    private static final int REQ_SAVE_AS = 1002;
    private static final int REQ_POST_NOTIFICATIONS = 1003;
    private static final int REQ_PICK_FONTS_DIR = 1004;
    private static final int REQ_PICK_IMAGE = 1005;
    private static final int REQ_PICK_UI_FONT = 1006;
    private static final long PALM_REJECTION_MS = 450L;
    private static final int MAX_STYLUS_QUEUE = 64;
    private static final int MAX_CLIPBOARD_TEXT_CHARS = 2 * 1024 * 1024;

    private static volatile WeakReference<FigCraftActivity> activeInstance = new WeakReference<>(null);
    private final ConcurrentLinkedQueue<String> controlQueue = new ConcurrentLinkedQueue<>();
    private final ConcurrentLinkedQueue<String> stylusQueue = new ConcurrentLinkedQueue<>();
    private final AtomicInteger stylusQueueSize = new AtomicInteger(0);

    private volatile boolean stylusActive = false;
    private volatile long lastStylusUptimeMs = 0L;
    private volatile boolean immersiveEnabled = false;
    private ImeBridgeView imeBridgeView;

    static {
        try {
            System.loadLibrary("c++_shared");
        } catch (UnsatisfiedLinkError ignored) {
        }
        System.loadLibrary("figcraft_android");
    }

    public static void notifyMcpStopped() {
        FigCraftActivity act = activeInstance.get();
        if (act != null) {
            act.enqueueControl("mcp_stopped");
        }
    }

    private void enqueueControl(String line) {
        controlQueue.add(line);
    }

    private void enqueueStylus(String line) {
        while (stylusQueueSize.get() >= MAX_STYLUS_QUEUE) {
            if (stylusQueue.poll() != null) {
                stylusQueueSize.decrementAndGet();
            } else {
                break;
            }
        }
        stylusQueue.add(line);
        stylusQueueSize.incrementAndGet();
    }

    private String pollEvent() {
        String ctrl = controlQueue.poll();
        if (ctrl != null) {
            return ctrl;
        }
        String stylus = stylusQueue.poll();
        if (stylus != null) {
            stylusQueueSize.decrementAndGet();
        }
        return stylus;
    }

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        activeInstance = new WeakReference<>(this);
        copyBundledUiFontIfPresent();
        enqueueControl("files_dir" + SEP + getFilesDir().getAbsolutePath());
        boolean isNight = (getResources().getConfiguration().uiMode & Configuration.UI_MODE_NIGHT_MASK) == Configuration.UI_MODE_NIGHT_YES;
        applySystemBarIconContrast(!isNight);
        enqueueControl("system_theme" + SEP + (isNight ? "dark" : "light"));

        setupImeBridgeView();
        View rootView = findViewById(android.R.id.content);
        if (rootView == null && getWindow() != null) {
            rootView = getWindow().getDecorView();
        }
        if (rootView != null) {
            ViewCompat.setOnApplyWindowInsetsListener(rootView, (v, insets) -> {
                if (immersiveEnabled) {
                    Insets cutout = insets.getInsets(WindowInsetsCompat.Type.displayCutout());
                    enqueueControl("insets" + SEP + cutout.top + SEP + cutout.right + SEP + cutout.bottom + SEP + cutout.left);
                } else {
                    Insets sys = insets.getInsets(
                            WindowInsetsCompat.Type.systemBars() | WindowInsetsCompat.Type.displayCutout()
                    );
                    enqueueControl("insets" + SEP + sys.top + SEP + sys.right + SEP + sys.bottom + SEP + sys.left);
                }

                Insets ime = insets.getInsets(WindowInsetsCompat.Type.ime());
                boolean imeVisible = insets.isVisible(WindowInsetsCompat.Type.ime());
                enqueueControl("ime" + SEP + (imeVisible ? "1" : "0") + SEP + ime.bottom);
                return ViewCompat.onApplyWindowInsets(v, insets);
            });
            ViewCompat.requestApplyInsets(rootView);
            setupCrossAppDragAndDrop(rootView);
        }
    }

    @Override
    public void onWindowFocusChanged(boolean hasFocus) {
        super.onWindowFocusChanged(hasFocus);
        if (hasFocus && immersiveEnabled) {
            applyImmersiveMode(true);
        }
    }

    @Override
    public void onConfigurationChanged(Configuration newConfig) {
        super.onConfigurationChanged(newConfig);
        boolean isNight = (newConfig.uiMode & Configuration.UI_MODE_NIGHT_MASK) == Configuration.UI_MODE_NIGHT_YES;
        enqueueControl("system_theme" + SEP + (isNight ? "dark" : "light"));
        View decor = getWindow().getDecorView();
        if (decor != null) {
            decor.post(() -> ViewCompat.requestApplyInsets(decor));
        }
    }

    @Override
    protected void onPause() {
        enqueueControl("app_paused");
        super.onPause();
    }

    @Override
    protected void onDestroy() {
        if (isFinishing()) {
            stopService(new Intent(this, McpForegroundService.class));
            if (activeInstance.get() == this) {
                activeInstance = new WeakReference<>(null);
            }
        }
        super.onDestroy();
    }

    private void applySystemBarIconContrast(boolean isLightTheme) {
        try {
            if (getWindow() == null) return;
            View decor = getWindow().getDecorView();
            if (decor == null) return;
            WindowInsetsControllerCompat controller = WindowCompat.getInsetsController(getWindow(), decor);
            if (controller != null) {
                controller.setAppearanceLightStatusBars(isLightTheme);
                controller.setAppearanceLightNavigationBars(isLightTheme);
            }
        } catch (Exception ignored) {
        }
    }

    private void applyImmersiveMode(boolean enable) {
        immersiveEnabled = enable;
        try {
            if (getWindow() == null) return;
            View decor = getWindow().getDecorView();
            if (decor == null) return;
            WindowCompat.setDecorFitsSystemWindows(getWindow(), !enable);
            WindowInsetsControllerCompat controller = WindowCompat.getInsetsController(getWindow(), decor);
            if (controller != null) {
                if (enable) {
                    controller.setSystemBarsBehavior(
                            WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
                    );
                    controller.hide(WindowInsetsCompat.Type.systemBars());
                } else {
                    controller.show(WindowInsetsCompat.Type.systemBars());
                }
            }
            ViewCompat.requestApplyInsets(decor);
        } catch (Exception ignored) {
        }
    }

    @Override
    public boolean dispatchTouchEvent(MotionEvent ev) {
        boolean hasStylus = forwardAllStylusPointers(ev);
        if (!hasStylus && isPalmRejectionActive()) {
            return true;
        }
        return super.dispatchTouchEvent(ev);
    }

    @Override
    public boolean dispatchGenericMotionEvent(MotionEvent ev) {
        forwardAllStylusPointers(ev);
        return super.dispatchGenericMotionEvent(ev);
    }

    private boolean isPalmRejectionActive() {
        if (stylusActive) {
            return true;
        }
        long elapsed = SystemClock.uptimeMillis() - lastStylusUptimeMs;
        return lastStylusUptimeMs > 0L && elapsed >= 0L && elapsed < PALM_REJECTION_MS;
    }

    private boolean forwardAllStylusPointers(MotionEvent ev) {
        int count = ev.getPointerCount();
        int masked = ev.getActionMasked();
        int actionIdx = ev.getActionIndex();
        boolean foundStylus = false;

        for (int i = 0; i < count; i++) {
            int tool = ev.getToolType(i);
            if (tool != MotionEvent.TOOL_TYPE_STYLUS && tool != MotionEvent.TOOL_TYPE_ERASER) {
                continue;
            }
            foundStylus = true;
            lastStylusUptimeMs = SystemClock.uptimeMillis();

            String action;
            if (masked == MotionEvent.ACTION_MOVE) {
                action = "move";
                stylusActive = true;
            } else if ((masked == MotionEvent.ACTION_DOWN || masked == MotionEvent.ACTION_POINTER_DOWN) && i == actionIdx) {
                action = "down";
                stylusActive = true;
            } else if ((masked == MotionEvent.ACTION_UP || masked == MotionEvent.ACTION_POINTER_UP || masked == MotionEvent.ACTION_CANCEL) && i == actionIdx) {
                action = "up";
                stylusActive = false;
            } else if (masked == MotionEvent.ACTION_HOVER_EXIT) {
                action = "up";
                stylusActive = false;
            } else {
                action = "hover";
                stylusActive = true;
            }

            float x = ev.getX(i);
            float y = ev.getY(i);
            float pressure = ev.getPressure(i);
            float tilt = ev.getAxisValue(MotionEvent.AXIS_TILT, i);
            float orientation = ev.getOrientation(i);
            float tiltX = (float) (Math.sin(orientation) * tilt);
            float tiltY = (float) (Math.cos(orientation) * tilt);
            boolean isEraser = (tool == MotionEvent.TOOL_TYPE_ERASER);

            enqueueStylus(
                    "stylus" + SEP + action + SEP + x + SEP + y + SEP + pressure + SEP
                            + tiltX + SEP + tiltY + SEP + (isEraser ? "1" : "0")
            );
        }
        return foundStylus;
    }

    @SuppressWarnings("unused")
    public String figcraftCall(String cmd, String arg) {
        try {
            switch (cmd) {
                case "poll": {
                    String next = pollEvent();
                    return next == null ? "ok" : ("ok" + SEP + next);
                }
                case "set_immersive": {
                    boolean enable = "1".equals(arg);
                    runOnUiThread(() -> applyImmersiveMode(enable));
                    return "ok";
                }
                case "set_light_bars": {
                    boolean isLight = "1".equals(arg);
                    runOnUiThread(() -> applySystemBarIconContrast(isLight));
                    return "ok";
                }
                case "open_file": {
                    runOnUiThread(() -> {
                        try {
                            Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
                            intent.addCategory(Intent.CATEGORY_OPENABLE);
                            intent.setType("*/*");
                            intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION
                                    | Intent.FLAG_GRANT_WRITE_URI_PERMISSION
                                    | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
                            startActivityForResult(intent, REQ_OPEN_FILE);
                        } catch (Exception ignored) {
                        }
                    });
                    return "ok";
                }
                case "pick_image": {
                    runOnUiThread(() -> {
                        try {
                            Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
                            intent.addCategory(Intent.CATEGORY_OPENABLE);
                            intent.setType("image/*");
                            intent.putExtra(Intent.EXTRA_MIME_TYPES, new String[]{
                                    "image/png", "image/jpeg", "image/webp", "image/svg+xml"
                            });
                            intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
                            startActivityForResult(intent, REQ_PICK_IMAGE);
                        } catch (Exception ignored) {
                        }
                    });
                    return "ok";
                }
                case "save_as": {
                    String[] parts = arg.split(SEP, -1);
                    String name = parts.length > 0 && !parts[0].isEmpty() ? parts[0] : "design.figcraft";
                    String mime = parts.length > 1 && !parts[1].isEmpty() ? parts[1] : "application/octet-stream";
                    runOnUiThread(() -> {
                        try {
                            Intent intent = new Intent(Intent.ACTION_CREATE_DOCUMENT);
                            intent.addCategory(Intent.CATEGORY_OPENABLE);
                            intent.setType(mime);
                            intent.putExtra(Intent.EXTRA_TITLE, name);
                            intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION
                                    | Intent.FLAG_GRANT_WRITE_URI_PERMISSION
                                    | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
                            startActivityForResult(intent, REQ_SAVE_AS);
                        } catch (Exception ignored) {
                        }
                    });
                    return "ok";
                }
                case "pick_ui_font": {
                    runOnUiThread(() -> {
                        try {
                            Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
                            intent.addCategory(Intent.CATEGORY_OPENABLE);
                            intent.setType("*/*");
                            intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
                            startActivityForResult(intent, REQ_PICK_UI_FONT);
                        } catch (Exception ignored) {
                        }
                    });
                    return "ok";
                }
                case "pick_fonts_dir": {
                    runOnUiThread(() -> {
                        try {
                            Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE);
                            intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION
                                    | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
                            startActivityForResult(intent, REQ_PICK_FONTS_DIR);
                        } catch (Exception ignored) {
                        }
                    });
                    return "ok";
                }
                case "save_mediastore": {
                    String[] parts = arg.split(SEP, -1);
                    if (parts.length < 4) return "err" + SEP + "invalid save_mediastore args";
                    saveToMediaStore(parts[0], parts[1], parts[2], new File(parts[3]));
                    return "ok";
                }
                case "write_uri": {
                    String[] parts = arg.split(SEP, -1);
                    if (parts.length < 2) return "err" + SEP + "invalid write_uri args";
                    copyFileToUri(new File(parts[1]), Uri.parse(parts[0]));
                    return "ok";
                }
                case "soft_keyboard": {
                    boolean show = "1".equals(arg);
                    runOnUiThread(() -> toggleSoftKeyboard(show));
                    return "ok";
                }
                case "set_clipboard": {
                    runOnUiThread(() -> {
                        ClipboardManager cm = (ClipboardManager) getSystemService(Context.CLIPBOARD_SERVICE);
                        if (cm != null) {
                            cm.setPrimaryClip(ClipData.newPlainText("FigCraft", arg));
                        }
                    });
                    return "ok";
                }
                case "paste_clipboard": {
                    runOnUiThread(this::readPrimaryClipboardAsync);
                    return "ok";
                }
                case "open_url": {
                    runOnUiThread(() -> {
                        try {
                            Intent browser = new Intent(Intent.ACTION_VIEW, Uri.parse(arg));
                            startActivity(browser);
                        } catch (Exception ignored) {
                        }
                    });
                    return "ok";
                }
                case "get_local_ip": {
                    String ip = findLocalIpv4();
                    return ip == null ? "ok" : ("ok" + SEP + ip);
                }
                case "start_mcp_service": {
                    runOnUiThread(() -> {
                        try {
                            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU
                                    && ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS)
                                    != PackageManager.PERMISSION_GRANTED) {
                                ActivityCompat.requestPermissions(
                                        this,
                                        new String[]{Manifest.permission.POST_NOTIFICATIONS},
                                        REQ_POST_NOTIFICATIONS
                                );
                            }
                            Intent svc = new Intent(this, McpForegroundService.class);
                            svc.putExtra(McpForegroundService.EXTRA_ENDPOINT, arg);
                            ContextCompat.startForegroundService(this, svc);
                        } catch (Exception ignored) {
                            // Keep embedded HTTP server running without crashing if FGS is restricted
                        }
                    });
                    return "ok";
                }
                case "stop_mcp_service": {
                    stopService(new Intent(this, McpForegroundService.class));
                    return "ok";
                }
                case "save_crash_log": {
                    FigCraftApp.reportNativeError(this, arg);
                    return "ok";
                }
                default:
                    return "err" + SEP + "unknown cmd: " + cmd;
            }
        } catch (Exception e) {
            return "err" + SEP + e.getMessage();
        }
    }

    private void readPrimaryClipboardAsync() {
        ClipboardManager cm = (ClipboardManager) getSystemService(Context.CLIPBOARD_SERVICE);
        if (cm == null || !cm.hasPrimaryClip()) return;
        ClipData clip = cm.getPrimaryClip();
        if (clip == null || clip.getItemCount() == 0) return;

        ClipData.Item item = clip.getItemAt(0);
        Uri uri = item.getUri();
        if (uri != null) {
            String mime = getContentResolver().getType(uri);
            if (mime == null) {
                ClipDescription desc = clip.getDescription();
                if (desc != null && desc.getMimeTypeCount() > 0) {
                    for (int i = 0; i < desc.getMimeTypeCount(); i++) {
                        String m = desc.getMimeType(i);
                        if (m != null && m.startsWith("image/")) {
                            mime = m;
                            break;
                        }
                    }
                }
            }
            if (mime != null && mime.startsWith("image/")) {
                final String finalMime = mime;
                new Thread(() -> {
                    try {
                        File staged = new File(getCacheDir(), "clip_" + System.currentTimeMillis() + ".img");
                        try (InputStream in = getContentResolver().openInputStream(uri);
                             OutputStream out = new FileOutputStream(staged)) {
                            if (in != null) {
                                copyStream(in, out);
                                enqueueControl("clipboard_image" + SEP + staged.getAbsolutePath() + SEP + finalMime);
                            }
                        }
                    } catch (Exception ignored) {
                    }
                }).start();
                return;
            }
        }

        CharSequence text = item.coerceToText(this);
        if (text != null && text.length() > 0) {
            String str = text.length() > MAX_CLIPBOARD_TEXT_CHARS
                    ? text.subSequence(0, MAX_CLIPBOARD_TEXT_CHARS).toString()
                    : text.toString();
            enqueueControl("clipboard_text" + SEP + str);
        }
    }

    private void setupCrossAppDragAndDrop(View target) {
        target.setOnDragListener((v, event) -> {
            if (event == null) return false;
            switch (event.getAction()) {
                case DragEvent.ACTION_DRAG_STARTED:
                    return true;
                case DragEvent.ACTION_DROP:
                    handleCrossAppDrop(event);
                    return true;
                default:
                    return true;
            }
        });
    }

    private void handleCrossAppDrop(DragEvent event) {
        ClipData clip = event.getClipData();
        if (clip == null || clip.getItemCount() == 0) return;

        final DragAndDropPermissions perms = requestDragAndDropPermissions(event);
        ClipDescription desc = clip.getDescription();

        boolean spawnedAsync = false;
        for (int i = 0; i < clip.getItemCount(); i++) {
            ClipData.Item item = clip.getItemAt(i);
            if (item == null) continue;
            Uri uri = item.getUri();
            if (uri != null) {
                String mime = getContentResolver().getType(uri);
                if (mime == null && desc != null) {
                    for (int m = 0; m < desc.getMimeTypeCount(); m++) {
                        String cand = desc.getMimeType(m);
                        if (cand != null && cand.startsWith("image/")) {
                            mime = cand;
                            break;
                        }
                    }
                }
                if (mime != null && mime.startsWith("image/")) {
                    spawnedAsync = true;
                    final String finalMime = mime;
                    final Uri finalUri = uri;
                    final int idx = i;
                    new Thread(() -> {
                        try {
                            File staged = new File(getCacheDir(), "drop_" + System.currentTimeMillis() + "_" + idx + ".img");
                            try (InputStream in = getContentResolver().openInputStream(finalUri);
                                 OutputStream out = new FileOutputStream(staged)) {
                                if (in != null) {
                                    copyStream(in, out);
                                    enqueueControl("clipboard_image" + SEP + staged.getAbsolutePath() + SEP + finalMime);
                                }
                            }
                        } catch (Exception ignored) {
                        } finally {
                            if (perms != null) {
                                try {
                                    perms.release();
                                } catch (Exception ignored) {
                                }
                            }
                        }
                    }).start();
                    continue;
                }
            }

            CharSequence text = item.coerceToText(this);
            if (text != null && text.length() > 0) {
                String str = text.length() > MAX_CLIPBOARD_TEXT_CHARS
                        ? text.subSequence(0, MAX_CLIPBOARD_TEXT_CHARS).toString()
                        : text.toString();
                enqueueControl("clipboard_text" + SEP + str);
            }
        }

        if (!spawnedAsync && perms != null) {
            try {
                perms.release();
            } catch (Exception ignored) {
            }
        }
    }

    private void setupImeBridgeView() {
        try {
            ViewGroup content = findViewById(android.R.id.content);
            if (content == null || imeBridgeView != null) return;
            imeBridgeView = new ImeBridgeView(this);
            ViewGroup.LayoutParams lp = new ViewGroup.LayoutParams(1, 1);
            content.addView(imeBridgeView, lp);
        } catch (Exception ignored) {
        }
    }

    private void toggleSoftKeyboard(boolean show) {
        try {
            InputMethodManager imm = (InputMethodManager) getSystemService(Context.INPUT_METHOD_SERVICE);
            if (imm == null) return;
            if (imeBridgeView == null) {
                setupImeBridgeView();
            }
            View target = imeBridgeView != null ? imeBridgeView : getWindow().getDecorView();
            if (target == null) return;
            if (show) {
                target.setFocusable(true);
                target.setFocusableInTouchMode(true);
                target.requestFocus();
                if (imeBridgeView != null) {
                    imeBridgeView.resetComposing();
                }
                imm.showSoftInput(target, InputMethodManager.SHOW_IMPLICIT);
                enqueueControl("ime_state" + SEP + "1");
            } else {
                imm.hideSoftInputFromWindow(target.getWindowToken(), 0);
                target.clearFocus();
                enqueueControl("ime_state" + SEP + "0");
            }
        } catch (Exception ignored) {
        }
    }

    /**
     * Hidden 1x1 native Android View providing a full BaseInputConnection so Korean IME
     * composition, English, numbers, symbols, Backspace, and Enter are reliably delivered to egui.
     */
    private class ImeBridgeView extends View {
        private String lastComposing = "";

        ImeBridgeView(Context context) {
            super(context);
            setFocusable(true);
            setFocusableInTouchMode(true);
            setAlpha(0f);
        }

        void resetComposing() {
            lastComposing = "";
        }

        @Override
        public boolean onCheckIsTextEditor() {
            return true;
        }

        @Override
        public InputConnection onCreateInputConnection(EditorInfo outAttrs) {
            outAttrs.inputType = InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS;
            outAttrs.imeOptions = EditorInfo.IME_FLAG_NO_FULLSCREEN | EditorInfo.IME_ACTION_DONE;
            return new BaseInputConnection(this, true) {
                @Override
                public boolean setComposingText(CharSequence text, int newCursorPosition) {
                    String s = text != null ? text.toString() : "";
                    lastComposing = s;
                    enqueueControl("ime_preedit" + SEP + s);
                    return super.setComposingText(text, newCursorPosition);
                }

                @Override
                public boolean commitText(CharSequence text, int newCursorPosition) {
                    String s = text != null ? text.toString() : "";
                    lastComposing = "";
                    if (!s.isEmpty()) {
                        enqueueControl("ime_commit" + SEP + s);
                    }
                    Editable ed = getEditable();
                    if (ed != null && ed.length() > 64) {
                        ed.clear();
                    }
                    return super.commitText(text, newCursorPosition);
                }

                @Override
                public boolean finishComposingText() {
                    if (!lastComposing.isEmpty()) {
                        String committed = lastComposing;
                        lastComposing = "";
                        enqueueControl("ime_commit" + SEP + committed);
                    }
                    return super.finishComposingText();
                }

                @Override
                public boolean deleteSurroundingText(int beforeLength, int afterLength) {
                    if (!lastComposing.isEmpty()) {
                        lastComposing = "";
                        enqueueControl("ime_preedit" + SEP);
                    }
                    int count = Math.max(0, Math.min(beforeLength, 16));
                    for (int i = 0; i < count; i++) {
                        enqueueControl("ime_key" + SEP + "backspace");
                    }
                    return super.deleteSurroundingText(beforeLength, afterLength);
                }

                @Override
                public boolean performEditorAction(int editorAction) {
                    if (!lastComposing.isEmpty()) {
                        String committed = lastComposing;
                        lastComposing = "";
                        enqueueControl("ime_commit" + SEP + committed);
                    }
                    enqueueControl("ime_key" + SEP + "enter");
                    return true;
                }

                @Override
                public boolean sendKeyEvent(KeyEvent event) {
                    if (event != null && event.getAction() == KeyEvent.ACTION_DOWN) {
                        int code = event.getKeyCode();
                        if (code == KeyEvent.KEYCODE_DEL) {
                            enqueueControl("ime_key" + SEP + "backspace");
                            return true;
                        } else if (code == KeyEvent.KEYCODE_ENTER || code == KeyEvent.KEYCODE_NUMPAD_ENTER) {
                            if (!lastComposing.isEmpty()) {
                                String committed = lastComposing;
                                lastComposing = "";
                                enqueueControl("ime_commit" + SEP + committed);
                            }
                            enqueueControl("ime_key" + SEP + "enter");
                            return true;
                        } else if (code == KeyEvent.KEYCODE_TAB) {
                            enqueueControl("ime_key" + SEP + "tab");
                            return true;
                        } else if (code == KeyEvent.KEYCODE_ESCAPE) {
                            enqueueControl("ime_key" + SEP + "escape");
                            return true;
                        } else {
                            int ch = event.getUnicodeChar();
                            if (ch >= 32) {
                                enqueueControl("ime_commit" + SEP + String.valueOf((char) ch));
                                return true;
                            }
                        }
                    }
                    return super.sendKeyEvent(event);
                }
            };
        }
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (resultCode != RESULT_OK || data == null || data.getData() == null) {
            return;
        }
        Uri uri = data.getData();

        int takeFlags = data.getFlags()
                & (Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION);
        if (takeFlags != 0) {
            try {
                getContentResolver().takePersistableUriPermission(uri, takeFlags);
            } catch (SecurityException ignored) {
            }
        }

        if (requestCode == REQ_OPEN_FILE) {
            String displayName = queryDisplayName(uri);
            String safeName = sanitizeFilename(displayName);
            new Thread(() -> {
                try {
                    File staged = new File(getCacheDir(), "saf_" + System.currentTimeMillis() + "_" + safeName);
                    try (InputStream in = getContentResolver().openInputStream(uri);
                         OutputStream out = new FileOutputStream(staged)) {
                        if (in != null) copyStream(in, out);
                    }
                    enqueueControl("file_opened" + SEP + staged.getAbsolutePath() + SEP + displayName);
                } catch (Exception ignored) {
                }
            }).start();
        } else if (requestCode == REQ_PICK_IMAGE) {
            String displayName = queryDisplayName(uri);
            String safeName = sanitizeFilename(displayName);
            String mime = getContentResolver().getType(uri);
            final String finalMime = mime != null ? mime : "image/png";
            new Thread(() -> {
                try {
                    File staged = new File(getCacheDir(), "img_" + System.currentTimeMillis() + "_" + safeName);
                    try (InputStream in = getContentResolver().openInputStream(uri);
                         OutputStream out = new FileOutputStream(staged)) {
                        if (in != null) copyStream(in, out);
                    }
                    enqueueControl("image_picked" + SEP + staged.getAbsolutePath() + SEP + displayName + SEP + finalMime);
                } catch (Exception ignored) {
                }
            }).start();
        } else if (requestCode == REQ_SAVE_AS) {
            String displayName = queryDisplayName(uri);
            enqueueControl("save_as" + SEP + uri.toString() + SEP + displayName);
        } else if (requestCode == REQ_PICK_FONTS_DIR) {
            new Thread(() -> {
                int count = mirrorFontTreeToPrivateDir(uri);
                enqueueControl("fonts_synced" + SEP + count);
            }).start();
        } else if (requestCode == REQ_PICK_UI_FONT) {
            String displayName = queryDisplayName(uri);
            new Thread(() -> {
                try {
                    File dst = new File(getFilesDir(), "ui_font.custom");
                    try (InputStream in = getContentResolver().openInputStream(uri);
                         OutputStream out = new FileOutputStream(dst)) {
                        if (in != null) copyStream(in, out);
                    }
                    enqueueControl("ui_font_picked" + SEP + dst.getAbsolutePath() + SEP + displayName);
                } catch (Exception ignored) {
                }
            }).start();
        }
    }

    private void copyBundledUiFontIfPresent() {
        try {
            File dst = new File(getFilesDir(), "bundled_ui_font.otf");
            if (dst.exists() && dst.length() > 10000) return;
            try (InputStream in = getAssets().open("ui_font.otf");
                 OutputStream out = new FileOutputStream(dst)) {
                copyStream(in, out);
            }
        } catch (Exception ignored) {
        }
    }

    private int mirrorFontTreeToPrivateDir(Uri treeUri) {
        File destDir = new File(getFilesDir(), "fonts");
        if (!destDir.exists()) {
            //noinspection ResultOfMethodCallIgnored
            destDir.mkdirs();
        }
        int copied = 0;
        String docId = DocumentsContract.getTreeDocumentId(treeUri);
        Uri childrenUri = DocumentsContract.buildChildDocumentsUriUsingTree(treeUri, docId);
        String[] projection = new String[]{
                DocumentsContract.Document.COLUMN_DOCUMENT_ID,
                DocumentsContract.Document.COLUMN_DISPLAY_NAME,
                DocumentsContract.Document.COLUMN_MIME_TYPE
        };
        try (Cursor c = getContentResolver().query(childrenUri, projection, null, null, null)) {
            if (c == null) return 0;
            while (c.moveToNext()) {
                String childId = c.getString(0);
                String name = c.getString(1);
                if (name == null) continue;
                String lower = name.toLowerCase(Locale.US);
                if (!(lower.endsWith(".ttf") || lower.endsWith(".otf") || lower.endsWith(".ttc") || lower.endsWith(".otc"))) {
                    continue;
                }
                Uri fileUri = DocumentsContract.buildDocumentUriUsingTree(treeUri, childId);
                File outFile = new File(destDir, sanitizeFilename(name));
                try (InputStream in = getContentResolver().openInputStream(fileUri);
                     OutputStream out = new FileOutputStream(outFile)) {
                    if (in != null) {
                        copyStream(in, out);
                        copied++;
                    }
                } catch (Exception ignored) {
                }
            }
        } catch (Exception ignored) {
        }
        return copied;
    }

    private static String sanitizeFilename(String name) {
        if (name == null || name.isEmpty()) return "file.figcraft";
        String cleaned = name.replaceAll("[^a-zA-Z0-9._-]", "_");
        return cleaned.isEmpty() ? "file.figcraft" : cleaned;
    }

    private void saveToMediaStore(String subdir, String filename, String mime, File source) throws Exception {
        ContentValues values = new ContentValues();
        values.put(MediaStore.Downloads.DISPLAY_NAME, filename);
        values.put(MediaStore.Downloads.MIME_TYPE, mime);
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            values.put(MediaStore.Downloads.RELATIVE_PATH,
                    Environment.DIRECTORY_DOWNLOADS + "/FigCraft/" + subdir);
            values.put(MediaStore.Downloads.IS_PENDING, 1);
        }
        Uri target = getContentResolver().insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values);
        if (target == null) {
            throw new IllegalStateException("MediaStore insert returned null");
        }
        try {
            copyFileToUri(source, target);
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                ContentValues done = new ContentValues();
                done.put(MediaStore.Downloads.IS_PENDING, 0);
                getContentResolver().update(target, done, null, null);
            }
        } catch (Exception e) {
            getContentResolver().delete(target, null, null);
            throw e;
        }
    }

    private void copyFileToUri(File source, Uri target) throws Exception {
        OutputStream out;
        try {
            out = getContentResolver().openOutputStream(target, "wt");
        } catch (IllegalArgumentException | UnsupportedOperationException e) {
            out = getContentResolver().openOutputStream(target, "w");
        }
        if (out == null) throw new IllegalStateException("openOutputStream returned null");
        try (InputStream in = new FileInputStream(source); OutputStream closableOut = out) {
            copyStream(in, closableOut);
        }
    }

    private static void copyStream(InputStream in, OutputStream out) throws Exception {
        byte[] buf = new byte[64 * 1024];
        int n;
        while ((n = in.read(buf)) > 0) {
            out.write(buf, 0, n);
        }
    }

    private String queryDisplayName(Uri uri) {
        try (Cursor c = getContentResolver().query(uri, null, null, null, null)) {
            if (c != null && c.moveToFirst()) {
                int idx = c.getColumnIndex(OpenableColumns.DISPLAY_NAME);
                if (idx >= 0) return c.getString(idx);
            }
        } catch (Exception ignored) {
        }
        return "untitled.figcraft";
    }

    private static String findLocalIpv4() {
        try {
            Enumeration<NetworkInterface> ifaces = NetworkInterface.getNetworkInterfaces();
            while (ifaces != null && ifaces.hasMoreElements()) {
                NetworkInterface iface = ifaces.nextElement();
                if (!iface.isUp() || iface.isLoopback()) continue;
                Enumeration<InetAddress> addrs = iface.getInetAddresses();
                while (addrs.hasMoreElements()) {
                    InetAddress addr = addrs.nextElement();
                    if (addr instanceof Inet4Address && !addr.isLoopbackAddress()) {
                        return addr.getHostAddress();
                    }
                }
            }
        } catch (Exception ignored) {
        }
        return null;
    }
}
