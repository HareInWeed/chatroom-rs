import React, { FC, useEffect, useState, useRef } from "react";
import Button from "@mui/material/Button";
import CssBaseline from "@mui/material/CssBaseline";
import TextField from "@mui/material/TextField";
import Box from "@mui/material/Box";
import { createTheme, ThemeProvider } from "@mui/material/styles";
import Table from "@mui/material/Table";
import TableBody from "@mui/material/TableBody";
import TableCell from "@mui/material/TableCell";
import TableContainer from "@mui/material/TableContainer";
import TableHead from "@mui/material/TableHead";
import TableRow from "@mui/material/TableRow";
import "./App.css";
import { invoke } from "@tauri-apps/api/tauri";
import { listen } from "@tauri-apps/api/event";

const theme = createTheme();

type ServerStatus = "running" | "ready" | "busy";

type UserInfo = [string, string | null];

const user_info_comparer = (a: UserInfo, b: UserInfo): number => {
  if (a[1] == null) {
    if (b[1] != null) {
      return 1;
    } else {
      return a[0].localeCompare(b[0]);
    }
  } else {
    if (b[1] == null) {
      return -1;
    } else {
      return a[0].localeCompare(b[0]);
    }
  }
};

const App: FC = () => {
  const bottomTag = useRef<HTMLSpanElement | null>(null);
  const [ip_addr, set_ip_addr] = useState("0.0.0.0:0");
  const [heartbeat_time, set_heartbeat_time] = useState("60000");
  const [server_status, set_server_status] = useState<ServerStatus>("ready");
  const [logs, set_logs] = useState<string[]>([]);
  const [user_info, set_user_info] = useState<UserInfo[]>([]);

  useEffect(() => {
    const unsubscribe = listen("log", (event) => {
      const log: string = (event as any).payload;
      set_logs((logs) => [...logs, log]);
      if (bottomTag.current != null) {
        bottomTag.current.scrollIntoView({ behavior: "smooth" });
      }
    });
    return () => {
      unsubscribe.then((f) => f());
    };
  });

  useEffect(() => {
    const unsubscribe = listen("user-info-updated", (log) => {
      (async () => {
        let data: {
          name: string;
          online_info: { ip_address: string } | null;
        }[] = await invoke("get_users");
        let new_user_info: UserInfo[] = data.map((u) => [
          u.name,
          u.online_info == null ? null : u.online_info.ip_address,
        ]);
        new_user_info.sort(user_info_comparer);
        set_user_info(new_user_info);
      })();
    });
    return () => {
      unsubscribe.then((f) => f());
    };
  });

  return (
    <ThemeProvider theme={theme}>
      <Box
        style={{
          display: "flex",
          flexDirection: "column",
          height: "100vh",
          maxHeight: "100vh",
          alignItems: "stretch",
        }}
      >
        <CssBaseline />
        <Box
          style={{
            display: "flex",
            flexDirection: "row",
            justifyContent: "flex-start",
            alignItems: "center",
            borderBottom: "2px solid #a5a5a5",
          }}
        >
          <TextField
            margin="normal"
            label="IP 地址"
            placeholder="默认自动选择地址与端口"
            value={ip_addr}
            onChange={(event) => set_ip_addr(event.target.value)}
            sx={{ ml: 2 }}
          />
          <TextField
            margin="normal"
            label="心跳信号超时时长"
            placeholder="单位（ms）"
            value={heartbeat_time}
            onChange={(event) => set_heartbeat_time(event.target.value)}
            sx={{ ml: 2 }}
          />
          <Button
            fullWidth
            variant="contained"
            sx={{
              width: "120px",
              height: "40px",
              ml: "auto",
              mt: "16px",
              mb: "8px",
              mr: 10,
            }}
            disabled={server_status === "busy"}
            onClick={async () => {
              if (server_status === "ready") {
                try {
                  console.log(heartbeat_time);
                  console.log(ip_addr === "" ? "0.0.0.0:0" : ip_addr);
                  await invoke("set_settings", {
                    heartbeatInterval:
                      heartbeat_time === ""
                        ? "60000"
                        : parseInt(heartbeat_time),
                    serverAddr: ip_addr === "" ? "0.0.0.0:0" : ip_addr,
                  });
                  set_server_status("busy");
                  await invoke("start_server");
                  set_server_status("running");
                } catch (err) {
                  console.error(err);
                  set_server_status("ready");
                }
              } else {
                set_server_status("busy");
                await invoke("stop_server");
                set_user_info([]);
                set_logs([]);
                set_server_status("ready");
              }
            }}
          >
            {
              { ready: "启动服务器", running: "停止服务器", busy: "正在启动" }[
                server_status
              ]
            }
          </Button>
        </Box>
        <Box
          style={{
            display: "flex",
            flexDirection: "row",
            flexGrow: 1,
            alignItems: "stretch",
          }}
        >
          <Box
            style={{
              display: "flex",
              flexDirection: "column",
              width: "50vw",
              borderRight: "2px solid #a5a5a5",
            }}
          >
            <Box sx={{ padding: "10px", borderBottom: "1px solid #c4c4c4" }}>
              日志
            </Box>
            <Box
              sx={{
                display: "flex",
                flexDirection: "column",
                width: "50vw",
                flexGrow: 1,
                alignItems: "stretch",

                fontFamily:
                  "source-code-pro, Menlo, Monaco, Consolas, 'Courier New', monospace, 'Segoe UI'",
                overflowX: "clip",
                overflowY: "auto",

                padding: "10px 10px 30px 10px",
                fontSize: "0.8em",
                lineHeight: "1em",

                overflowWrap: "break-word",
                maxHeight: "calc(100vh - 127px)",
              }}
            >
              {logs.map((log, id) => (
                <Box
                  key={id}
                  sx={{
                    margin: "3px 0",
                  }}
                >
                  {log}
                </Box>
              ))}
              <span id="bottom_tag" ref={bottomTag}></span>
            </Box>
          </Box>
          <Box
            style={{
              display: "flex",
              flexDirection: "column",
              width: "50vw",
            }}
          >
            <Box sx={{ padding: "10px", borderBottom: "1px solid #c4c4c4" }}>
              用户信息
            </Box>
            <Box
              style={{
                display: "flex",
                flexDirection: "row",
                flexGrow: 1,
                alignItems: "stretch",

                overflowX: "auto",
                overflowY: "auto",

                maxHeight: "calc(100vh - 127px)",
              }}
            >
              <TableContainer component={Box}>
                <Table sx={{ minWidth: 400 }} size="small">
                  <TableHead>
                    <TableRow>
                      <TableCell>用户名</TableCell>
                      <TableCell>IP</TableCell>
                    </TableRow>
                  </TableHead>
                  <TableBody>
                    {user_info.map((user) => (
                      <TableRow
                        key={user[0]}
                        sx={{
                          "&:last-child td, &:last-child th": { border: 0 },
                        }}
                      >
                        <TableCell>{user[0]}</TableCell>
                        <TableCell>{user[1] || "-"}</TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </TableContainer>
            </Box>
          </Box>
        </Box>
      </Box>
    </ThemeProvider>
  );
};

export default App;
