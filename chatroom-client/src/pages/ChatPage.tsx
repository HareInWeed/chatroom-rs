import React, { FC, useState, useEffect, useCallback, useRef } from "react";

import {
  Button,
  TextField,
  Link,
  Box,
  Typography,
  List,
  ListSubheader,
  ListItem,
  ListItemButton,
  ListItemIcon,
  ListItemText,
  Collapse,
  Modal,
} from "@mui/material";

import {
  ExpandLess,
  ExpandMore,
  ForumOutlined,
  Person,
  PersonOutline,
  Logout,
  Edit,
  Refresh,
} from "@mui/icons-material";

import ChangePassModal from "components/ChangePassModal";

import { invoke } from "@tauri-apps/api/tauri";
import { listen, Event as TauriEvent } from "@tauri-apps/api/event";

import { useSnackbar } from "notistack";

import { useContainer } from "unstated-next";
import PathState from "states/PathState";

interface BackendUser {
  name: string;
  online_info: null | {
    ip_address: string;
    pub_key: number[];
  };
}

interface User {
  name: string;
  is_online: boolean;
  new_msg_count: number;
}

type ChatData = [
  string,
  { user: string; entry: "Online" | "Offline" | { Message: string } }
];

const compare_users = (a: User, b: User) => {
  // first compare by have new message or not
  const no_msg_a = a.new_msg_count === 0 ? 0 : 1;
  const no_msg_b = b.new_msg_count === 0 ? 0 : 1;
  const cmp1 = [
    [0, 1],
    [-1, 0],
  ][no_msg_a][no_msg_b];
  if (cmp1 !== 0) {
    return cmp1;
  }

  // then compare by user is online or not
  const is_online_a = a.is_online ? 1 : 0;
  const is_online_b = a.is_online ? 1 : 0;
  const cmp2 = [
    [0, 1],
    [-1, 0],
  ][is_online_a][is_online_b];
  if (cmp2 !== 0) {
    return cmp2;
  }

  // then compare by number of messages
  const msg_count_diff = b.new_msg_count - a.new_msg_count;
  if (msg_count_diff !== 0) {
    return msg_count_diff;
  }

  // then compare by username
  return a.name.localeCompare(b.name);
};

const get_user_info = async (name: string | null): Promise<User[]> => {
  const raw_users = (await invoke("get_user_info")) as BackendUser[];
  const raw_users_without_me = raw_users.filter((u) => u.name !== name);
  const users_without_me = raw_users_without_me.map(
    ({ name, online_info }) => ({
      name,
      is_online: online_info != null,
      new_msg_count: 0,
    })
  );
  return users_without_me;
};

const ChatPage: FC = () => {
  const { set_path } = useContainer(PathState);
  const { enqueueSnackbar } = useSnackbar();

  const [server_addr, set_server_addr] = useState("");
  const [username, set_username] = useState("");
  const [users, set_users] = useState<Map<string, User>>(new Map());

  const [online_list, set_online_list] = useState<string[]>([]);
  const [offline_list, set_offline_list] = useState<string[]>([]);
  const [chat_history, set_chat_history] = useState<ChatData[]>([]);
  const [public_chat_count, set_public_chat_count] = useState<number>(0);

  const bottomTag = useRef<HTMLSpanElement | null>(null);
  const input_textarea = useRef<HTMLTextAreaElement>(null);
  const [_force_rerender, set_force_rerender] = useState(false); // eslint-disable no-unused-vars
  const [expand, set_expand] = useState<boolean>(false);
  const [modal_open, set_modal_open] = useState<boolean>(false);
  const [current_chat, set_current_chat] = useState<string | null>(null);
  const [message, set_message] = useState("");

  const force_rerender = useCallback(() => {
    set_force_rerender((b) => !b);
  }, []);

  const refresh_user_info = useCallback(async () => {
    const name = ((await invoke("get_personal_info")) as any).name;
    set_username(name);
    set_server_addr(await invoke("get_server_info"));
    const users = await get_user_info(name);
    set_users(new Map(users.map((u) => [u.name, u])));

    let online_users = users.filter((u) => u.is_online);
    let offline_users = users.filter((u) => !u.is_online);
    online_users.sort(compare_users);
    offline_users.sort(compare_users);
    set_online_list(online_users.map((u) => u.name));
    set_offline_list(offline_users.map((u) => u.name));
  }, []);

  const get_chat_history = useCallback(
    async (chat: string | null) => {
      try {
        const data: ChatData[] = await invoke("get_chats", { name: chat });
        set_chat_history(data);
        if (bottomTag.current != null) {
          bottomTag.current.scrollIntoView({
            behavior: chat === current_chat ? "smooth" : "auto",
          });
        }
      } catch (err) {
        const msg = (err as any).msg;
        if (typeof msg === "string") {
          if (msg === "user is not existed") {
            set_chat_history([]);
          } else {
            console.error(err);
          }
        } else {
          console.error(err);
        }
      }
    },
    [bottomTag, current_chat]
  );

  const select_chat = useCallback(
    async (chat_name: string | null) => {
      if (chat_name == null) {
        set_public_chat_count(0);
      } else {
        set_users((users) => {
          const user = users.get(chat_name);
          if (user != null) {
            user.new_msg_count = 0;
          }
          return new Map(users);
        });
      }
      set_current_chat(chat_name);
      await get_chat_history(chat_name);
    },
    [get_chat_history]
  );

  const bump_user_online = useCallback((name: string) => {
    set_offline_list((l) => l.filter((n) => n !== name));
    set_online_list((l) => [name, ...l.filter((n) => n !== name)]);
  }, []);

  const move_user_online = useCallback((name: string) => {
    set_offline_list((l) => l.filter((n) => n !== name));
    set_online_list((l) => [...l.filter((n) => n !== name), name]);
  }, []);

  const move_user_offline = useCallback((name: string) => {
    set_online_list((l) => l.filter((n) => n !== name));
    set_offline_list((l) => [...l.filter((n) => n !== name), name]);
  }, []);

  const set_online_state = useCallback(
    (name: string, is_online: boolean) => {
      const user = users.get(name);
      if (user != null) {
        user.is_online = is_online;
      } else {
        users.set(name, {
          name,
          is_online: true,
          new_msg_count: 0,
        });
      }
      set_users(new Map(users));
      if (is_online) {
        move_user_online(name);
      } else {
        move_user_offline(name);
      }
    },
    [users, move_user_online, move_user_offline]
  );

  const say = useCallback(async () => {
    if (/^\s*$/.test(message)) {
      enqueueSnackbar("不能发送空白消息", { variant: "warning" });
      return;
    }
    try {
      await invoke("say", { username: current_chat, msg: message });
      get_chat_history(current_chat);
      set_message("");
    } catch (err) {
      const msg = (err as any).msg;
      if (typeof msg === "string") {
        if (msg === "user is offline") {
          enqueueSnackbar("用户不在线", { variant: "error" });
        } else {
          console.error(err);
        }
      } else {
        console.error(err);
      }
    }
  }, [current_chat, message, get_chat_history, enqueueSnackbar]);

  // initialization
  useEffect(() => {
    (async () => {
      await refresh_user_info();
    })();
  }, [refresh_user_info]);

  // new message
  useEffect(() => {
    const unsubscribe = listen("new-msg", (data: TauriEvent<string | null>) => {
      (async () => {
        const chat = data.payload;
        if (chat === current_chat) {
          await get_chat_history(current_chat);
        } else if (chat == null) {
          set_public_chat_count((c) => c + 1);
        } else {
          const user = users.get(chat);
          if (user != null) {
            user.new_msg_count += 1;
            set_users(users);
            bump_user_online(chat);
          }
        }
      })();
    });
    return () => {
      unsubscribe.then((f) => f());
    };
  }, [current_chat, users, get_chat_history, bump_user_online]);

  // some user is online
  useEffect(() => {
    const unsubscribe = listen("online", (data: TauriEvent<string>) => {
      (async () => {
        set_online_state(data.payload, true);
      })();
    });
    return () => {
      unsubscribe.then((f) => f());
    };
  }, [set_online_state]);

  // some user is offline
  useEffect(() => {
    const unsubscribe = listen("offline", (data: TauriEvent<string>) => {
      (async () => {
        set_online_state(data.payload, false);
      })();
    });
    return () => {
      unsubscribe.then((f) => f());
    };
  }, [set_online_state]);

  // ctrl-enter listener
  useEffect(() => {
    let elem = input_textarea.current;
    if (elem != null) {
      const handler = (event: KeyboardEvent) => {
        if (event.ctrlKey && event.key === "Enter") {
          say();
        }
      };
      elem.addEventListener("keydown", handler);
      return () => {
        if (elem != null) {
          elem.removeEventListener("keydown", handler);
        }
      };
    }
  }, [say]);

  return (
    <Box
      sx={{
        display: "flex",
        flexDirection: "row",
        width: "100%",
        height: "100vh",
      }}
    >
      <Box>
        <List
          component="nav"
          sx={{
            height: "100vh",
            width: "200px",
            pt: 0,
            bgcolor: "background.paper",
            overflowY: "auto",
            borderRight: "1px solid #ccc",
            boxShadow: 5,

            "& .MuiListItemText-root": {
              width: "167px",
            },
            "& .MuiListItemText-root>.MuiTypography-root": {
              whiteSpace: "nowrap",
              overflow: "hidden",
              textOverflow: "ellipsis",
            },
          }}
        >
          <ListItem
            sx={{
              display: "flex",
              flexDirection: "column",
              alignItems: "flex-start",
              pl: 2,
              "&>*": {
                width: "167px",
                display: "inline-block",
                whiteSpace: "nowrap",
                overflow: "hidden",
                textOverflow: "ellipsis",
              },
            }}
          >
            <Typography
              variant="h6"
              component="div"
              sx={{ lineHeight: "0.9em", pt: 1 }}
            >
              {server_addr}
            </Typography>
            <Typography
              variant="body1"
              gutterBottom
              sx={{
                ml: "6px",
                color: "#999",
              }}
            >
              {username}
            </Typography>
          </ListItem>
          <ListSubheader
            component="div"
            sx={{ display: "flex" }}
            onClick={() => set_expand((b) => !b)}
          >
            操作
            <Box sx={{ ml: "auto", display: "flex", alignItems: "center" }}>
              {expand ? <ExpandLess /> : <ExpandMore />}
            </Box>
          </ListSubheader>
          <Collapse in={expand} timeout="auto" unmountOnExit>
            <List component="div" disablePadding>
              <ListItemButton
                onClick={async () => {
                  try {
                    await invoke("logout");
                    set_path(["connection"]);
                    enqueueSnackbar(`您已成功登出服务器 ${server_addr}`, {
                      variant: "success",
                    });
                  } catch (err) {
                    console.error(err);
                  }
                }}
              >
                <ListItemIcon>
                  <Logout />
                </ListItemIcon>
                <ListItemText primary="登出" />
              </ListItemButton>
              <ListItemButton onClick={() => set_modal_open(true)}>
                <ListItemIcon>
                  <Edit />
                </ListItemIcon>
                <ListItemText primary="修改密码" />
              </ListItemButton>
              <ListItemButton
                onClick={async () => {
                  try {
                    await invoke("fetch_chatroom_status");
                    await refresh_user_info();
                    enqueueSnackbar("已刷新用户信息", { variant: "success" });
                  } catch (err) {
                    console.error(err);
                  }
                }}
              >
                <ListItemIcon>
                  <Refresh />
                </ListItemIcon>
                <ListItemText primary="刷新用户信息" />
              </ListItemButton>
            </List>
          </Collapse>

          <ListSubheader component="div">聊天</ListSubheader>
          <ListItemButton
            sx={{ backgroundColor: current_chat == null ? "#ddd" : "#fff" }}
            onClick={() => select_chat(null)}
          >
            <ListItemIcon>
              <ForumOutlined />
              {public_chat_count !== 0 && (
                <RedDot>
                  {public_chat_count < 999 ? public_chat_count : "999+"}
                </RedDot>
              )}
            </ListItemIcon>
            <ListItemText primary="公共群聊" />
          </ListItemButton>
          {[...online_list, ...offline_list].map((name) => {
            const user = users.get(name);
            if (user != null) {
              return (
                <ListItemButton
                  key={user.name}
                  onClick={() => select_chat(name)}
                  sx={{
                    backgroundColor: current_chat === name ? "#ddd" : "#fff",
                  }}
                >
                  <ListItemIcon>
                    {user.is_online ? (
                      <PersonOutline />
                    ) : (
                      <Person sx={{ color: "#ccccccaa" }} />
                    )}
                    {user.new_msg_count !== 0 && (
                      <RedDot>
                        {user.new_msg_count < 999 ? user.new_msg_count : "999+"}
                      </RedDot>
                    )}
                  </ListItemIcon>
                  <ListItemText primary={user.name} />
                </ListItemButton>
              );
            } else {
              return null;
            }
          })}
        </List>
      </Box>
      <Box
        sx={{
          display: "flex",
          flexDirection: "column",
          justifyContent: "center",
          alignItems: "stretch",
          flex: "1 0 auto",
        }}
      >
        <Box
          sx={{
            display: "flex",
            flexDirection: "column",
            alignItems: "stretch",
            height: "70vh",
            overflowY: "auto",
            py: 3,
          }}
        >
          {chat_history.map(([raw_time, chat]) => {
            // TODO: could be improved by formatting time at frontend
            const time = raw_time.slice(0, 19);
            if (chat.entry === "Online" || chat.entry === "Offline") {
              return (
                <Box
                  sx={{
                    marginTop: "10px",
                    alignSelf: "center",
                    fontSize: "0.9em",
                    backgroundColor: "#bfbfbf",
                    color: "#fff",
                    verticalAlign: "bottom",

                    display: "flex",
                    flexDirection: "row",
                    alignItems: "center",

                    px: 1,
                    borderRadius: 3,
                  }}
                >
                  用户 "
                  <Box
                    component="span"
                    sx={{
                      maxWidth: "100px",
                      display: "inline-block",
                      whiteSpace: "nowrap",
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                    }}
                  >
                    {chat.user}
                  </Box>
                  " 在 {time} {chat.entry === "Online" ? "进入" : "退出"}
                  了聊天室
                </Box>
              );
            } else if (chat.user === username) {
              return (
                <Box
                  sx={{
                    whiteSpace: "pre-wrap",
                    wordBreak: "break-all",
                    maxWidth: "calc(100vw - 400px)",
                    px: 2,
                    py: 1,
                    mt: 3,
                    borderRadius: 3,
                    boxShadow: 4,

                    display: "flex",
                    flexDirection: "column",

                    mr: 2,
                    alignSelf: "flex-end",
                    backgroundColor: "primary.dark",
                    color: "#fff",
                  }}
                >
                  <Typography variant="caption" sx={{ alignSelf: "flex-end" }}>
                    [{time}] 我
                  </Typography>
                  <Box sx={{ alignSelf: "flex-end" }}>{chat.entry.Message}</Box>
                </Box>
              );
            } else {
              return (
                <Box
                  sx={{
                    whiteSpace: "pre-wrap",
                    wordBreak: "break-all",
                    maxWidth: "calc(100vw - 400px)",
                    px: 2,
                    py: 1,
                    mt: 3,
                    borderRadius: 3,
                    boxShadow: 4,

                    display: "flex",
                    flexDirection: "column",

                    ml: 2,
                    alignSelf: "flex-start",
                  }}
                >
                  <Typography
                    variant="caption"
                    sx={{ alignSelf: "flex-start" }}
                  >
                    [{time}] {chat.user}
                  </Typography>
                  <Box sx={{ alignSelf: "flex-start" }}>
                    {chat.entry.Message}
                  </Box>
                </Box>
              );
            }
          })}
          <span id="bottom_tag" ref={bottomTag}></span>
        </Box>
        <Box
          sx={{
            height: "30vh",
            boxShadow: 5,
            display: "flex",
            flexDirection: "column",
          }}
        >
          <TextField
            multiline
            rows={5}
            placeholder="在这里输入你的消息，按下 Ctrl + Enter 或发送按钮发送"
            variant="standard"
            value={message}
            onChange={(event) => set_message(event.target.value)}
            sx={{
              p: 1,
              pb: 0,
              "& .MuiInput-root::before": { border: "none" },
              "& .MuiInput-root:hover::before": { border: "none !important" },
              "& .MuiInput-root::after": { border: "none" },
            }}
            inputRef={input_textarea}
          />
          <Button
            variant="contained"
            size="small"
            sx={{ my: "auto", mr: 5, alignSelf: "flex-end" }}
            onClick={() => say()}
          >
            发送
          </Button>
        </Box>
      </Box>
      <Modal open={modal_open} onClose={() => set_modal_open(false)}>
        <ChangePassModal onClose={() => set_modal_open(false)} />
      </Modal>
    </Box>
  );
};

const RedDot: FC = ({ children }) => (
  <Box sx={{ position: "relative", width: 0, height: 0 }}>
    <Box
      sx={{
        position: "absolute",
        left: "-10px",
        bottom: "-11px",

        minWidth: "20px",
        height: "20px",
        borderRadius: "10px",
        px: "5px",

        backgroundColor: "red",
        color: "white",
        fontSize: "0.5em",

        display: "flex",
        flexDirection: "column",
        justifyContent: "center",
        alignItems: "center",
      }}
    >
      <Box>{children}</Box>
    </Box>
  </Box>
);

export default ChatPage;
