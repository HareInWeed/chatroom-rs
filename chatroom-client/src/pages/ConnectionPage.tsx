import React, { FC } from "react";

import { AUTHOR } from "about";

import {
  Avatar,
  Button,
  TextField,
  Box,
  Typography,
  Container,
  CircularProgress,
} from "@mui/material";
import { Forum } from "@mui/icons-material";

import { invoke } from "@tauri-apps/api/tauri";

import { useForm, Controller } from "react-hook-form";

import { useSnackbar } from "notistack";

import { useContainer } from "unstated-next";
import PathState from "states/PathState";

type LoginData = {
  addr: string;
};

const ConnectionPage: FC = () => {
  const { set_path } = useContainer(PathState);
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
      maxWidth="sm"
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
        <Box
          sx={{ display: "flex", flexDirection: "row", alignItems: "center" }}
        >
          <Avatar
            sx={{
              m: 1,
              mr: 2,
              bgcolor: "secondary.main",
              width: 60,
              height: 60,
            }}
          >
            <Forum sx={{ fontSize: 40 }} />
          </Avatar>
          <Typography component="h1" variant="h5">
            UDP 聊天室
          </Typography>
        </Box>
        <Box
          component="form"
          onSubmit={handleSubmit(async (data) => {
            try {
              await invoke("connect_server", { serverAddr: data.addr });
              enqueueSnackbar(`成功连接服务器 ${data.addr}`, {
                variant: "success",
              });
              set_path(["login"]);
            } catch (err) {
              const msg = (err as any).msg;
              if (typeof msg === "string") {
                if (msg === "invalid IP address syntax") {
                  setError("addr", { message: "服务器地址不合法" });
                } else if (msg === "request timeout") {
                  setError("addr", { message: "连接服务器超时" });
                } else {
                  console.error(err);
                }
              } else {
                console.error(err);
              }
            }
          })}
          noValidate
          sx={{
            display: "flex",
            flexDirection: "row",
            alignSelf: "stretch",
            mt: 1,
          }}
        >
          <Controller
            name="addr"
            control={control}
            defaultValue=""
            rules={{ required: "请输入服务器地址" }}
            render={({
              field: { onChange, onBlur, value, ref },
              fieldState: { error },
              formState: { isSubmitting },
            }) => (
              <TextField
                id="addr"
                margin="normal"
                required
                fullWidth
                label="聊天室服务器地址"
                autoComplete="on"
                autoFocus
                error={!!error}
                helperText={error && error.message}
                disabled={isSubmitting}
                value={value}
                onChange={onChange}
                onBlur={onBlur}
                sx={{
                  flexGrow: 1,
                  "&>div": {
                    borderTopRightRadius: 0,
                    borderBottomRightRadius: 0,
                  },
                }}
                inputRef={ref}
              />
            )}
          />

          <Button
            type="submit"
            fullWidth
            variant="contained"
            sx={{
              mt: 2,
              mb: 1,
              width: "60px",
              height: "57px",
              borderTopLeftRadius: 0,
              borderBottomLeftRadius: 0,
              boxShadow: 0,
            }}
            disabled={isSubmitting}
          >
            {isSubmitting ? <CircularProgress size={20} /> : "连接"}
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

export default ConnectionPage;
