// mod group {
//     use super::{Any, Deserialize, GenResourceID, Postgres, Resource, Result, Serialize, Sqlite};
//
//     #[derive(Deserialize, Serialize, PartialEq, Debug, resource_macros::Resource)]
//     #[resource(
//         schema_name = "slep",
//         pg_table_name = "group",
//         sqlite_table_name = "group",
//         primary_key = "id:i64",
//         constraint = "slep_group_pkey"
//     )]
//     pub struct Group<'g> {
//         pub pid: Option<i64>,
//         pub name: &'g str,
//         pub des: &'g str,
//         pub timestamp: i64,
//     }
//
//     impl GenResourceID for Group<'_> {
//         type Target = i64;
//
//         async fn gen_id() -> Result<i64> {
//             todo!()
//         }
//     }
//
//     #[derive(Deserialize, Serialize, PartialEq, Debug, resource_macros::Resource)]
//     #[resource(
//         schema_name = "slep",
//         pg_table_name = "group_member",
//         sqlite_table_name = "group_member",
//         primary_key = "id:i64, gid:i64",
//         constraint = "slep_group_member_pkey"
//     )]
//     pub struct GroupMember {
//         level: i16,
//         timestamp: i64,
//     }
//
//     impl GenResourceID for GroupMember {
//         type Target = (i64, i64);
//
//         async fn gen_id() -> Result<(i64, i64)> {
//             todo!()
//         }
//     }
// }
//
// #[derive(Deserialize, Serialize, Debug)]
// enum TestResources<'a> {
//     #[serde(borrow)]
//     Message(Command<Postgres, GeneralAction<Postgres, message::Message<'a>>>),
//     #[serde(borrow)]
//     Group(Command<Postgres, GeneralAction<Postgres, group::Group<'a>>>),
//     GroupMember(Command<Postgres, GeneralAction<Postgres, group::GroupMember>>),
// }
//
// impl Resources for TestResources<'_> {}
//
// impl Action for TestResources<'_> {
//     async fn execute<'c, E>(&self, executor: E) -> Result<()>
//     where
//         E: SqlxExecutor<'c, Database = Any>,
//     {
//         match self {
//             TestResources::Message(r) => r.execute(executor).await,
//             TestResources::Group(r) => r.execute(executor).await,
//             TestResources::GroupMember(r) => r.execute(executor).await,
//         }
//     }
// }
#![feature(async_closure, associated_type_bounds, let_chains)]
#![allow(unused)]
pub use resource_macros;

use std::marker::PhantomData;

use anyhow::{Ok, Result};
use serde::{Deserialize, Serialize};

use sqlx::{database::Database as SqlxDatabase, Any, Executor as SqlxExecutor, Postgres, Sqlite};

pub trait Resources: Action {}

pub trait Action: Serialize {
    async fn execute<'c, E>(&self, executor: E) -> Result<()>
    where
        E: SqlxExecutor<'c, Database = Any>;
}

pub trait GenResourceID {
    type Target;

    async fn gen_id() -> Result<Self::Target>;
}

pub trait Resource<DB: SqlxDatabase>: GenResourceID<Target = Self::ResourceID> + Serialize {
    type ResourceID: Serialize;

    async fn insert<'c, E>(&self, id: &Option<Self::ResourceID>, executor: E) -> Result<()>
    where
        E: SqlxExecutor<'c, Database = Any>;

    async fn upsert<'c, E>(&self, id: &Option<Self::ResourceID>, executor: E) -> Result<()>
    where
        E: SqlxExecutor<'c, Database = Any>;

    async fn update<'c, E>(&self, id: &Self::ResourceID, executor: E) -> Result<()>
    where
        E: SqlxExecutor<'c, Database = Any>;

    async fn drop<'c, E>(id: &Self::ResourceID, executor: E) -> Result<()>
    where
        E: SqlxExecutor<'c, Database = Any>;
}

#[derive(Deserialize, Serialize, PartialEq, Debug)]
pub enum GeneralAction<DB: SqlxDatabase, R: Resource<DB>> {
    Insert {
        id: Option<R::ResourceID>,
        resource: R,
    },
    Upsert {
        id: Option<R::ResourceID>,
        resource: R,
    },
    Update {
        id: R::ResourceID,
        resource: R,
    },
    Drop(R::ResourceID),
}

impl<DB: SqlxDatabase, R: Resource<DB>> Action for GeneralAction<DB, R> {
    async fn execute<'c, E>(&self, executor: E) -> Result<()>
    where
        E: SqlxExecutor<'c, Database = Any>,
    {
        match self {
            GeneralAction::Insert { id, resource } => resource.insert(id, executor).await,
            GeneralAction::Upsert { id, resource } => resource.upsert(id, executor).await,
            GeneralAction::Update { id, resource } => resource.update(id, executor).await,
            GeneralAction::Drop(id) => R::drop(id, executor).await,
        }
    }
}

#[derive(Deserialize, Serialize, PartialEq, Debug, Clone)]
pub struct Command<A>
where
    A: Action,
{
    pub trace: i64,
    pub action: A,
    pub tag: String,
}

impl<A> Command<A>
where
    A: Action,
{
    pub fn new(trace: i64, action: A, tag: String) -> Command<A> {
        Command { trace, action, tag }
    }
}

impl<A> Action for Command<A>
where
    A: Action,
{
    async fn execute<'c, E>(&self, executor: E) -> Result<()>
    where
        E: SqlxExecutor<'c, Database = Any>,
    {
        self.action.execute(executor).await
    }
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(untagged)]
pub enum Commands<RS> {
    Single(RS),
    Multi(Vec<RS>),
}

impl<RS> Commands<RS>
where
    RS: Resources,
{
    #[allow(dead_code)]
    pub async fn execute<'e, 'c: 'e>(&self, pool: &'c sqlx::Pool<sqlx::Any>) -> Result<()> {
        match self {
            Commands::Single(cmd) => {
                cmd.execute(pool).await?;
            }
            Commands::Multi(cmds) => {
                let mut tx = pool.begin().await?;
                for cmd in cmds {
                    cmd.execute(&mut tx).await?;
                }
                tx.commit().await?;
            }
        };
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::{
        Any, Command, Deserialize, GenResourceID, GeneralAction, Postgres, Resource, Result,
        Serialize, Sqlite, SqlxDatabase, SqlxExecutor,
    };

    #[derive(Deserialize, Serialize, Debug)]
    enum Server<'a> {
        #[serde(borrow)]
        Message(Command<GeneralAction<Postgres, Message<'a>>>),
    }

    #[derive(Deserialize, Serialize, Debug)]
    enum Client<'a> {
        #[serde(borrow)]
        Message(Command<GeneralAction<Sqlite, Message<'a>>>),
    }

    #[derive(Deserialize, Serialize, PartialEq, Debug, resource_macros::Resource)]
    #[resource(
        schema_name = "slep",
        pg_table_name = "message",
        sqlite_table_name = "message",
        primary_key = "id:i64",
        constraint = "slep_message_pkey"
    )]
    pub struct Message<'m> {
        #[serde(borrow)]
        #[resource(typ = "slep.message_type")]
        pub typ: &'m str,
        #[serde(borrow)]
        #[resource(typ = "slep.message_addr_type")]
        pub addr_typ: &'m str,
        pub addr: i64,
        #[serde(borrow)]
        pub stream: &'m str,
        #[serde(borrow)]
        pub topic: &'m str,
        #[serde(borrow)]
        pub message_type: &'m str,
        #[serde(borrow)]
        pub content: &'m str,
        pub sender: i64,
        pub receiver: Option<i64>,
        pub timestamp: i64,
    }

    impl GenResourceID for Message<'_> {
        type Target = i64;

        async fn gen_id() -> Result<i64> {
            todo!()
        }
    }

    #[test]
    fn command_serde() {
        let m = Message {
            typ: "typ",
            addr_typ: "addr_typ",
            addr: 0,
            topic: "topic",
            content: "content",
            sender: 1111,
            receiver: None,
            timestamp: 0,
            message_type: "message_type",
            stream: "stream",
        };

        let action = GeneralAction::Upsert {
            id: None,
            resource: m,
        };
        let cmd = Command {
            trace: 0,
            action,
            tag: "Send".to_string(),
        };

        let resources = Server::Message(cmd);
        let resources_str = serde_json::to_string(&resources).unwrap();
        println!("res: {resources_str}");
        let res: Server = serde_json::from_str(&resources_str).unwrap();
        let res_str = serde_json::to_string(&res).unwrap();

        assert_eq!(resources_str, res_str);
    }

    #[test]
    fn server_to_client() {
        let m = Message {
            typ: "typ",
            addr_typ: "addr_typ",
            addr: 0,
            topic: "topic",
            content: "content",
            sender: 1111,
            receiver: None,
            timestamp: 0,
            message_type: "message_type",
            stream: "stream",
        };

        let action = GeneralAction::Upsert {
            id: None,
            resource: m,
        };
        let cmd = Command {
            trace: 0,
            action,
            tag: "Send".to_string(),
        };

        let server = Server::Message(cmd);
        let server_str = serde_json::to_string(&server).unwrap();
        let client: Client = serde_json::from_str(&server_str).unwrap();
        println!("client: {client:?}");
    }
}
