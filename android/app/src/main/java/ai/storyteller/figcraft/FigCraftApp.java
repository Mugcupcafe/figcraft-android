package ai.storyteller.figcraft;

import android.app.Application;
import android.content.ContentResolver;
import android.content.ContentValues;
import android.content.Context;
import android.content.Intent;
import android.net.Uri;
import android.os.Build;
import android.os.Environment;
import android.provider.MediaStore;

import java.io.File;
import java.io.FileOutputStream;
import java.io.OutputStream;
import java.io.PrintWriter;
import java.io.StringWriter;
import java.nio.charset.StandardCharsets;
import java.text.SimpleDateFormat;
import java.util.Arrays;
import java.util.Date;
import java.util.Locale;

/**
 * Installs a process-wide UncaughtExceptionHandler before GameActivity loads,
 * saves crash logs directly to Downloads/FigCraft_crash_*.txt (no permission needed on Android 10+),
 * and opens CrashReportActivity in a separate process so the user can view/copy the log on-device.
 */
public class FigCraftApp extends Application {
    private static volatile Context appContext;

    @Override
    public void onCreate() {
        super.onCreate();
        appContext = getApplicationContext();

        final Thread.UncaughtExceptionHandler previous = Thread.getDefaultUncaughtExceptionHandler();
        Thread.setDefaultUncaughtExceptionHandler((thread, throwable) -> {
            try {
                String report = buildReport("UNCAUGHT_JAVA_EXCEPTION", thread.getName(), throwable, null);
                String savedPath = saveLogToDownloads(appContext, report);
                launchCrashScreen(appContext, report, savedPath);
            } catch (Throwable ignored) {
                if (previous != null) {
                    previous.uncaughtException(thread, throwable);
                }
            }
            android.os.Process.killProcess(android.os.Process.myPid());
            System.exit(10);
        });
    }

    public static void reportNativeError(Context ctx, String detail) {
        Context c = ctx != null ? ctx.getApplicationContext() : appContext;
        if (c == null) return;
        String report = buildReport("RUST_NATIVE_ERROR", Thread.currentThread().getName(), null, detail);
        String savedPath = saveLogToDownloads(c, report);
        launchCrashScreen(c, report, savedPath);
    }

    private static String buildReport(String kind, String threadName, Throwable t, String extra) {
        StringBuilder sb = new StringBuilder();
        sb.append("=== FigCraft Android Crash Report ===\n");
        sb.append("Time: ").append(new SimpleDateFormat("yyyy-MM-dd HH:mm:ss", Locale.US).format(new Date())).append("\n");
        sb.append("Kind: ").append(kind).append("\n");
        sb.append("Thread: ").append(threadName).append("\n");
        sb.append("Device: ").append(Build.MANUFACTURER).append(" ").append(Build.MODEL).append("\n");
        sb.append("Android: ").append(Build.VERSION.RELEASE).append(" (API ").append(Build.VERSION.SDK_INT).append(")\n");
        sb.append("ABIs: ").append(Arrays.toString(Build.SUPPORTED_ABIS)).append("\n\n");
        if (extra != null && !extra.isEmpty()) {
            sb.append(extra).append("\n\n");
        }
        if (t != null) {
            StringWriter sw = new StringWriter();
            PrintWriter pw = new PrintWriter(sw);
            t.printStackTrace(pw);
            pw.flush();
            sb.append(sw.toString());
        }
        return sb.toString();
    }

    public static String saveLogToDownloads(Context ctx, String content) {
        String ts = new SimpleDateFormat("yyyyMMdd_HHmmss", Locale.US).format(new Date());
        String filename = "FigCraft_crash_" + ts + ".txt";
        byte[] bytes = content.getBytes(StandardCharsets.UTF_8);

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            try {
                ContentResolver resolver = ctx.getContentResolver();
                ContentValues values = new ContentValues();
                values.put(MediaStore.Downloads.DISPLAY_NAME, filename);
                values.put(MediaStore.Downloads.MIME_TYPE, "text/plain");
                values.put(MediaStore.Downloads.RELATIVE_PATH, Environment.DIRECTORY_DOWNLOADS);
                values.put(MediaStore.Downloads.IS_PENDING, 1);

                Uri uri = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values);
                if (uri != null) {
                    try (OutputStream out = resolver.openOutputStream(uri, "wt")) {
                        if (out != null) {
                            out.write(bytes);
                            out.flush();
                        }
                    }
                    values.clear();
                    values.put(MediaStore.Downloads.IS_PENDING, 0);
                    resolver.update(uri, values, null, null);
                    return "Downloads/" + filename;
                }
            } catch (Throwable ignored) {
            }
        }

        try {
            File dir = ctx.getExternalFilesDir(null);
            if (dir == null) dir = ctx.getFilesDir();
            File f = new File(dir, filename);
            try (FileOutputStream fos = new FileOutputStream(f)) {
                fos.write(bytes);
                fos.flush();
            }
            return f.getAbsolutePath();
        } catch (Throwable ignored) {
            return "(save failed)";
        }
    }

    private static void launchCrashScreen(Context ctx, String report, String savedPath) {
        try {
            Intent intent = new Intent(ctx, CrashReportActivity.class);
            intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_CLEAR_TASK);
            intent.putExtra(CrashReportActivity.EXTRA_REPORT, report);
            intent.putExtra(CrashReportActivity.EXTRA_SAVED_PATH, savedPath);
            ctx.startActivity(intent);
        } catch (Throwable ignored) {
        }
    }
}
