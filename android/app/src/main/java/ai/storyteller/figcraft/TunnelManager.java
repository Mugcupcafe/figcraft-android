package ai.storyteller.figcraft;

import android.content.Context;
import android.util.Log;

import org.json.JSONObject;

import java.io.BufferedReader;
import java.io.File;
import java.io.FileOutputStream;
import java.io.InputStreamReader;
import java.net.Inet4Address;
import java.net.InetAddress;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

import javax.net.ssl.HttpsURLConnection;

/**
 * Android-native Cloudflare Tunnel launcher (Quick Tunnel + Named Tunnel).
 * Resolves Cloudflare edge IPv4 in Java (Android Bionic DNS) and passes --edge <ip>:7844
 * to bundled libcloudflared.so inside nativeLibraryDir so Go never needs /etc/resolv.conf.
 */
public final class TunnelManager {
    private static final String TAG = "FigCraftTunnel";
    private static final String FALLBACK_CF_EDGE_IP = "198.41.192.77";

    public interface Listener {
        void onTunnelState(String state, String url, String message);
    }

    private final Context appContext;
    private final Listener listener;
    private final ExecutorService executor = Executors.newSingleThreadExecutor();
    private volatile Process currentProcess;
    private volatile boolean stopping = false;

    public TunnelManager(Context context, Listener listener) {
        this.appContext = context.getApplicationContext();
        this.listener = listener;
    }

    public void startCloudflareQuick(final int localPort) {
        stopInternal(false);
        stopping = false;
        listener.onTunnelState("connecting", "", "Cloudflare 퀵터널 발급 중...");
        executor.execute(() -> {
            try {
                File bin = findCloudflaredBinary();
                if (bin == null) {
                    listener.onTunnelState("error", "", "libcloudflared.so를 찾을 수 없습니다");
                    return;
                }

                // 1. Request ephemeral Quick Tunnel credentials via Android's native HTTPS/DNS stack
                URL apiUrl = new URL("https://api.trycloudflare.com/tunnel");
                HttpsURLConnection conn = (HttpsURLConnection) apiUrl.openConnection();
                conn.setRequestMethod("POST");
                conn.setConnectTimeout(12000);
                conn.setReadTimeout(12000);
                conn.setRequestProperty("Accept", "application/json");
                conn.setRequestProperty("User-Agent", "FigCraft-Mobile/28");
                conn.setDoOutput(true);
                conn.getOutputStream().close();

                int code = conn.getResponseCode();
                BufferedReader br = new BufferedReader(new InputStreamReader(
                        code >= 200 && code < 300 ? conn.getInputStream() : conn.getErrorStream(),
                        StandardCharsets.UTF_8));
                StringBuilder sb = new StringBuilder();
                String line;
                while ((line = br.readLine()) != null) sb.append(line);
                br.close();
                conn.disconnect();

                if (code < 200 || code >= 300) {
                    listener.onTunnelState("error", "", "퀵터널 발급 실패 (HTTP " + code + ")");
                    return;
                }

                JSONObject root = new JSONObject(sb.toString());
                JSONObject res = root.getJSONObject("result");
                String tunnelId = res.getString("id");
                String hostname = res.getString("hostname");
                String accountTag = res.getString("account_tag");
                String secret = res.getString("secret");
                String mcpUrl = "https://" + hostname + "/mcp";

                // 2. Write credentials & ingress config under app filesDir
                File tunnelDir = new File(appContext.getFilesDir(), "tunnel");
                if (!tunnelDir.exists()) tunnelDir.mkdirs();
                File credsFile = new File(tunnelDir, "cf_quick_creds.json");
                File cfgFile = new File(tunnelDir, "cf_quick_config.yml");

                JSONObject credsJson = new JSONObject();
                credsJson.put("AccountTag", accountTag);
                credsJson.put("TunnelSecret", secret);
                credsJson.put("TunnelID", tunnelId);
                writeUtf8(credsFile, credsJson.toString());

                String yaml = "tunnel: " + tunnelId + "\n"
                        + "credentials-file: " + credsFile.getAbsolutePath() + "\n"
                        + "ingress:\n"
                        + "  - service: http://127.0.0.1:" + localPort + "\n";
                writeUtf8(cfgFile, yaml);

                // 3. Resolve Cloudflare edge IPv4 in Java (bypasses Go /etc/resolv.conf on Android)
                String edgeIp = resolveCloudflareEdgeIpv4();
                listener.onTunnelState("connecting", mcpUrl, "Cloudflare 엣지(" + edgeIp + ") 연결 중...");

                ProcessBuilder pb = new ProcessBuilder(
                        bin.getAbsolutePath(),
                        "tunnel",
                        "--no-autoupdate",
                        "--protocol", "http2",
                        "--edge-ip-version", "4",
                        "--edge", edgeIp + ":7844",
                        "--config", cfgFile.getAbsolutePath(),
                        "run", tunnelId
                );
                pb.redirectErrorStream(true);
                runCloudflaredProcess(pb, mcpUrl);
            } catch (Exception e) {
                Log.e(TAG, "Cloudflare Quick Tunnel failed", e);
                if (!stopping) {
                    listener.onTunnelState("error", "", "퀵터널 오류: " + e.getMessage());
                }
            }
        });
    }

    public void startCloudflareNamed(final String token, final String customDomain) {
        stopInternal(false);
        stopping = false;
        final String cleanToken = token == null ? "" : token.trim();
        if (cleanToken.isEmpty()) {
            listener.onTunnelState("error", "", "Cloudflare 터널 토큰을 먼저 입력해 주세요");
            return;
        }
        String displayUrl = "";
        if (customDomain != null && !customDomain.trim().isEmpty()) {
            String d = customDomain.trim();
            if (!d.startsWith("http://") && !d.startsWith("https://")) {
                d = "https://" + d;
            }
            if (!d.endsWith("/mcp")) {
                d = d.endsWith("/") ? d + "mcp" : d + "/mcp";
            }
            displayUrl = d;
        }
        final String finalUrl = displayUrl;
        listener.onTunnelState("connecting", finalUrl, "Cloudflare 고정터널 연결 중...");
        executor.execute(() -> {
            try {
                File bin = findCloudflaredBinary();
                if (bin == null) {
                    listener.onTunnelState("error", "", "libcloudflared.so를 찾을 수 없습니다");
                    return;
                }
                String edgeIp = resolveCloudflareEdgeIpv4();
                ProcessBuilder pb = new ProcessBuilder(
                        bin.getAbsolutePath(),
                        "tunnel",
                        "--no-autoupdate",
                        "--protocol", "http2",
                        "--edge-ip-version", "4",
                        "--edge", edgeIp + ":7844",
                        "run",
                        "--token", cleanToken
                );
                pb.redirectErrorStream(true);
                runCloudflaredProcess(pb, finalUrl);
            } catch (Exception e) {
                Log.e(TAG, "Cloudflare Named Tunnel failed", e);
                if (!stopping) {
                    listener.onTunnelState("error", "", "고정터널 오류: " + e.getMessage());
                }
            }
        });
    }

    public void stop() {
        stopInternal(true);
    }

    private void stopInternal(boolean notify) {
        stopping = true;
        Process p = currentProcess;
        currentProcess = null;
        if (p != null) {
            try {
                p.destroy();
            } catch (Throwable ignored) {}
        }
        if (notify) {
            listener.onTunnelState("stopped", "", "터널이 중지되었습니다");
        }
    }

    private void runCloudflaredProcess(ProcessBuilder pb, String mcpUrl) throws Exception {
        Process proc = pb.start();
        currentProcess = proc;
        boolean connected = false;
        String lastErr = "";
        try (BufferedReader reader = new BufferedReader(
                new InputStreamReader(proc.getInputStream(), StandardCharsets.UTF_8))) {
            String line;
            while ((line = reader.readLine()) != null) {
                Log.i(TAG, "[cloudflared] " + line);
                if (!connected && (line.contains("Registered tunnel connection")
                        || line.contains("Connection registered"))) {
                    connected = true;
                    String msg = mcpUrl.isEmpty() ? "Cloudflare 고정터널 연결됨" : "연결 완료";
                    listener.onTunnelState("connected", mcpUrl, msg);
                } else if (line.contains("ERR ") || line.contains("failed") || line.contains("error=")) {
                    lastErr = line.length() > 120 ? line.substring(0, 120) : line;
                }
            }
        }
        int exit = proc.waitFor();
        if (currentProcess == proc) {
            currentProcess = null;
        }
        if (!stopping) {
            String msg = lastErr.isEmpty() ? ("터널 종료 (코드 " + exit + ")") : lastErr;
            listener.onTunnelState("error", "", msg);
        }
    }

    private File findCloudflaredBinary() {
        String libDir = appContext.getApplicationInfo().nativeLibraryDir;
        if (libDir == null) return null;
        File f = new File(libDir, "libcloudflared.so");
        return f.exists() ? f : null;
    }

    private String resolveCloudflareEdgeIpv4() {
        String[] hosts = new String[] {
                "region1.v2.argotunnel.com",
                "region2.v2.argotunnel.com"
        };
        for (String h : hosts) {
            try {
                InetAddress[] addrs = InetAddress.getAllByName(h);
                for (InetAddress a : addrs) {
                    if (a instanceof Inet4Address) {
                        return a.getHostAddress();
                    }
                }
            } catch (Throwable t) {
                Log.w(TAG, "DNS lookup failed for " + h + ": " + t.getMessage());
            }
        }
        return FALLBACK_CF_EDGE_IP;
    }

    private static void writeUtf8(File file, String content) throws Exception {
        try (FileOutputStream fos = new FileOutputStream(file)) {
            fos.write(content.getBytes(StandardCharsets.UTF_8));
        }
    }
}
