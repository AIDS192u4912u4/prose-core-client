use chrono::Utc;
use jid::BareJid;

use prose_core_client::test_helpers::{
    BareJidTestAdditions, ClientTestAdditions, ConnectedClient, DateTimeTestAdditions,
    MessageBuilder,
};
use prose_core_client::{Client, MessageCache};
use prose_core_lib::stanza::IQ as IQStanza;
use prose_core_lib::stanza::{Namespace, StanzaBase};
use prose_core_lib::test_helpers::StrExt;

#[tokio::test]
async fn test_loads_latest_messages_with_empty_cache() -> anyhow::Result<()> {
    let ConnectedClient {
        client, connection, ..
    } = Client::connected_client().await?;

    connection.set_stanza_handler(move |_| {
        vec![
            MessageBuilder::new_with_index(1).build_mam_message("id-1"),
            MessageBuilder::new_with_index(2).build_mam_message("id-1"),
            IQStanza::mam_end_marker("id-2", 1, 2, false).stanza_owned(),
        ]
    });

    let messages = client
        .load_latest_messages(&BareJid::theirs(), None, true)
        .await?;

    assert_eq!(
        messages,
        vec![
            MessageBuilder::new_with_index(1).build_message(),
            MessageBuilder::new_with_index(2).build_message()
        ]
    );

    let sent_stanzas = connection.sent_stanza_strings();
    assert_eq!(sent_stanzas.len(), 1);
    assert_eq!(
        sent_stanzas[0],
        r#"<iq id="id-2" type="set">
        <query queryid="id-1" xmlns="urn:xmpp:mam:2">
            <x xmlns="jabber:x:data" type="submit">
                <field type="hidden" var="FORM_TYPE">
                    <value>urn:xmpp:mam:2</value>
                </field>
                <field var="with">
                    <value>friend@prose.org</value>
                </field>
            </x>
            <set xmlns="http://jabber.org/protocol/rsm">
                <max>50</max>
                <before/>
            </set>
        </query>
    </iq>"#
            .to_xml_result_string()
    );

    Ok(())
}

#[tokio::test]
async fn test_loads_latest_messages_with_partial_cache() -> anyhow::Result<()> {
    let ConnectedClient {
        client,
        connection,
        data_cache,
        ..
    } = Client::connected_client().await?;

    data_cache.insert_messages(vec![
        &MessageBuilder::new_with_index(1)
            .set_timestamp(Utc::test_timestamp_adding(1))
            .build_message_like(),
        &MessageBuilder::new_with_index(2)
            .set_timestamp(Utc::test_timestamp_adding(2))
            .build_message_like(),
    ])?;

    connection.set_stanza_handler(move |_| {
        vec![
            MessageBuilder::new_with_index(3)
                .set_timestamp(Utc::test_timestamp_adding(3))
                .build_mam_message("id-1"),
            MessageBuilder::new_with_index(4)
                .set_timestamp(Utc::test_timestamp_adding(4))
                .build_mam_message("id-1"),
            IQStanza::mam_end_marker("id-2", 3, 4, true).stanza_owned(),
        ]
    });

    let messages = client
        .load_latest_messages(&BareJid::theirs(), None, true)
        .await?;

    assert_eq!(
        messages,
        vec![
            MessageBuilder::new_with_index(1)
                .set_timestamp(Utc::test_timestamp_adding(1))
                .build_message(),
            MessageBuilder::new_with_index(2)
                .set_timestamp(Utc::test_timestamp_adding(2))
                .build_message(),
            MessageBuilder::new_with_index(3)
                .set_timestamp(Utc::test_timestamp_adding(3))
                .build_message(),
            MessageBuilder::new_with_index(4)
                .set_timestamp(Utc::test_timestamp_adding(4))
                .build_message(),
        ]
    );

    let sent_stanzas = connection.sent_stanza_strings();
    assert_eq!(sent_stanzas.len(), 1);
    assert_eq!(
        sent_stanzas[0],
        r#"<iq id="id-2" type="set">
        <query queryid="id-1" xmlns="urn:xmpp:mam:2">
            <x xmlns="jabber:x:data" type="submit">
                <field type="hidden" var="FORM_TYPE">
                    <value>urn:xmpp:mam:2</value>
                </field>
                <field var="with">
                    <value>friend@prose.org</value>
                </field>
            </x>
            <set xmlns="http://jabber.org/protocol/rsm">
                <max>50</max>
                <after>res-2</after>
            </set>
        </query>
    </iq>"#
            .to_xml_result_string()
    );

    Ok(())
}

trait IQTestAdditions {
    fn mam_end_marker(
        id: impl Into<prose_core_lib::stanza::iq::Id>,
        first_message_idx: u32,
        last_message_idx: u32,
        complete: bool,
    ) -> prose_core_lib::stanza::IQ<'static>;
}

impl IQTestAdditions for IQStanza<'_> {
    fn mam_end_marker(
        id: impl Into<prose_core_lib::stanza::iq::Id>,
        first_message_idx: u32,
        last_message_idx: u32,
        complete: bool,
    ) -> prose_core_lib::stanza::IQ<'static> {
        use prose_core_lib::stanza;

        stanza::IQ::new(stanza::iq::Kind::Result, id).add_child(
            stanza::Stanza::new("fin")
                .set_namespace(Namespace::MAM2)
                .add_child(
                    stanza::Stanza::new("set")
                        .set_attribute::<&str>(
                            "complete",
                            if complete { Some("true") } else { None },
                        )
                        .set_namespace(Namespace::RSM)
                        .add_child(stanza::Stanza::new_text_node(
                            "first",
                            format!("res-{}", first_message_idx),
                        ))
                        .add_child(stanza::Stanza::new_text_node(
                            "last",
                            format!("res-{}", last_message_idx),
                        )),
                ),
        )
    }
}
