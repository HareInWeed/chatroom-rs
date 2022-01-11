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
import { Create } from "@mui/icons-material";

import { invoke } from "@tauri-apps/api/tauri";

import { useForm, Controller } from "react-hook-form";

import { useSnackbar } from "notistack";

import { useContainer } from "unstated-next";
import PathState from "states/PathState";

type LoginData = {
  username: string;
  password: string;
};

const RegisterPage: FC = () => {
  let { set_path } = useContainer(PathState);
  const { enqueueSnackbar } = useSnackbar();

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
          <Create sx={{ fontSize: 40 }} />
        </Avatar>
        <Typography component="h1" variant="h5">
          注册账号
        </Typography>
        <Box
          component="form"
          onSubmit={handleSubmit(async (data) => {
            try {
              await invoke("register", {
                username: data.username,
                password: data.password,
              });
              enqueueSnackbar("注册成功，请重新登录账号", {
                variant: "success",
              });
              set_path(["login"]);
            } catch (err) {
              const msg = (err as any).msg;
              if (typeof msg === "string") {
                if (msg === "user is already existed") {
                  setError("username", { message: "用户名已被占用" });
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
                set_path(["login"]);
              }}
              sx={{
                m: 1,
                textDecoration: "none",
              }}
            >
              已有账号？点击返回登录
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
                注册中...&nbsp;
                <CircularProgress size={20} />
              </>
            ) : (
              "注册"
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

export default RegisterPage;
