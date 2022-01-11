import React, { FC } from "react";

import { AUTHOR } from "about";

import {
  Avatar,
  Button,
  TextField,
  Link,
  Box,
  Typography,
  Container,
  CircularProgress,
} from "@mui/material";
import { Forum } from "@mui/icons-material";

import { invoke } from "@tauri-apps/api/tauri";

import { useForm, Controller } from "react-hook-form";

import { useContainer } from "unstated-next";
import PathState from "states/PathState";

type LoginData = {
  addr: string;
  username: string;
  password: string;
};

const LoginPage: FC = () => {
  let { set_path } = useContainer(PathState);

  const {
    handleSubmit,
    setError,
    control,
    formState: { isSubmitting },
  } = useForm<LoginData>();

  return (
    <Container
      component="main"
      maxWidth="xs"
      sx={{
        flexGrow: 1,
        display: "flex",
        flexDirection: "column",
        justifyContent: "center",
        height: "100vh",
      }}
    >
      <Box
        sx={{
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          mb: 7,
        }}
      >
        <Avatar sx={{ m: 1, bgcolor: "secondary.main", width: 60, height: 60 }}>
          <Forum sx={{ fontSize: 40 }} />
        </Avatar>
        <Typography component="h1" variant="h5">
          登录
        </Typography>
        <Box
          component="form"
          onSubmit={handleSubmit(async (data) => {
            try {
              await invoke("login", {
                username: data.username,
                password: data.password,
              });
              set_path(["chatroom"]);
            } catch (err) {
              const msg = (err as any).msg;
              if (typeof msg === "string") {
                if (msg === "username or password are invalid") {
                  setError("username", { message: "用户名或密码不正确" });
                  setError("password", { message: "用户名或密码不正确" });
                } else {
                  console.error(err);
                }
              } else {
                console.error(err);
              }
            }
          })}
          noValidate
          sx={{ mt: 1 }}
        >
          <Controller
            name="username"
            control={control}
            defaultValue=""
            rules={{ required: "请输入用户名" }}
            render={({
              field: { onChange, onBlur, value, ref },
              fieldState: { error },
              formState: { isSubmitting },
            }) => (
              <TextField
                id="username"
                margin="normal"
                required
                fullWidth
                label="用户名"
                autoComplete="username"
                error={!!error}
                helperText={error && error.message}
                disabled={isSubmitting}
                value={value}
                onChange={onChange}
                onBlur={onBlur}
                inputRef={ref}
              />
            )}
          />

          <Controller
            name="password"
            control={control}
            defaultValue=""
            rules={{ required: "请输入密码" }}
            render={({
              field: { onChange, onBlur, value, ref },
              fieldState: { error },
              formState: { isSubmitting },
            }) => (
              <TextField
                id="password"
                margin="normal"
                required
                fullWidth
                label="密码"
                type="password"
                autoComplete="current-password"
                error={!!error}
                helperText={error && error.message}
                disabled={isSubmitting}
                value={value}
                onChange={onChange}
                onBlur={onBlur}
                inputRef={ref}
              />
            )}
          />

          <Box
            sx={{
              display: "flex",
              flexDirection: "row",
              justifyContent: "flex-start",
            }}
          >
            <Link
              href="#"
              variant="body2"
              onClick={(event) => {
                event.preventDefault();
                set_path(["register"]);
              }}
              sx={{
                m: 1,
                textDecoration: "none",
              }}
            >
              还没有账号？点击注册
            </Link>
            <Link
              href="#"
              variant="body2"
              onClick={async (event) => {
                event.preventDefault();
                try {
                  await invoke("disconnect_server");
                  set_path(["connection"]);
                } catch (err) {
                  console.error(err);
                }
              }}
              sx={{
                m: 1,
                ml: "auto",
                textDecoration: "none",
              }}
            >
              断开连接
            </Link>
          </Box>

          <Button
            type="submit"
            fullWidth
            variant="contained"
            sx={{ mt: 3, mb: 2 }}
            disabled={isSubmitting}
          >
            {isSubmitting ? (
              <>
                登录中...&nbsp;
                <CircularProgress size={20} />
              </>
            ) : (
              "登录"
            )}
          </Button>
        </Box>
        <Typography
          variant="body2"
          color="text.secondary"
          align="center"
          sx={{ marginTop: "30px" }}
        >
          By {AUTHOR}
        </Typography>
      </Box>
    </Container>
  );
};

export default LoginPage;
