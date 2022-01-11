import React, { FC, useEffect } from "react";
import "./App.css";
import { CssBaseline, Box, createTheme, ThemeProvider } from "@mui/material";

import { SnackbarProvider, useSnackbar } from "notistack";

import SnackbarCloseButton from "components/SnackBarCloseButton";

import ConnectionPage from "pages/ConnectionPage";
import LoginPage from "pages/LoginPage";
import RegisterPage from "pages/RegisterPage";
import ChatPage from "pages/ChatPage";

import { useContainer } from "unstated-next";
import PathState from "states/PathState";

import { listen } from "@tauri-apps/api/event";

const theme = createTheme();

const App: FC = () => {
  return (
    <ThemeProvider theme={theme}>
      <PathState.Provider initialState={["connection"]}>
        <SnackbarProvider
          anchorOrigin={{
            vertical: "bottom",
            horizontal: "left",
          }}
          autoHideDuration={3000}
          action={(snackbarKey) => (
            <SnackbarCloseButton snackbarKey={snackbarKey} />
          )}
          // sadly, notistack v2 doesn't work well with material-ui v5
          // TransitionComponent={Fade}
        >
          <CssBaseline />
          <AppDisplay />
        </SnackbarProvider>
      </PathState.Provider>
    </ThemeProvider>
  );
};

const AppDisplay: FC = () => {
  let { path, set_path } = useContainer(PathState);
  let { enqueueSnackbar } = useSnackbar();

  useEffect(() => {
    const unsubscribe = listen("connection-lost", () => {
      (async () => {
        enqueueSnackbar("服务器链接已断开", { variant: "error" });
        set_path(["connection"]);
      })();
    });
    return () => {
      unsubscribe.then((f) => f());
    };
  });

  useEffect(() => {
    const unsubscribe = listen("not-login", () => {
      (async () => {
        enqueueSnackbar("客户端未登录，请重新登录", { variant: "error" });
        set_path((path) => (path[0] !== "connection" ? ["login"] : path));
      })();
    });
    return () => {
      unsubscribe.then((f) => f());
    };
  });

  if (path[0] === "connection") {
    return <ConnectionPage />;
  } else if (path[0] === "login") {
    return <LoginPage />;
  } else if (path[0] === "register") {
    return <RegisterPage />;
  } else if (path[0] === "chatroom") {
    return <ChatPage />;
  } else {
    return <Box>routing error</Box>;
  }
};

export default App;
